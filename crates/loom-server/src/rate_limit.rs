//! In-process, per-key token-bucket rate limiting.
//!
//! Each virtual key gets two independent token buckets — one metering
//! requests-per-minute, one metering tokens-per-minute. A bucket starts full,
//! refills continuously at `capacity / 60` units per second, and a request is
//! admitted only if it can draw what it needs.
//!
//! # Known limitation: single-instance only
//!
//! The buckets live in this process's memory, so the limits are enforced
//! **per replica**: two gateway replicas each admit up to the configured rate.
//! Distributed rate limiting (a shared Redis token bucket) is deferred; see the
//! README. For a single-instance deployment this is exact.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use uuid::Uuid;

use loom_store::RateLimit;

/// A continuously-refilling token bucket.
#[derive(Debug)]
struct Bucket {
    /// Currently available tokens (fractional; refills continuously).
    available: f64,
    /// The bucket's maximum capacity (the per-minute limit).
    capacity: f64,
    /// Tokens added per second (`capacity / 60`).
    refill_per_sec: f64,
    /// When `available` was last brought up to date.
    last: Instant,
}

impl Bucket {
    /// A full bucket for a given per-minute `capacity`.
    fn new(capacity: f64, now: Instant) -> Self {
        Self {
            available: capacity,
            capacity,
            refill_per_sec: capacity / 60.0,
            last: now,
        }
    }

    /// Brings `available` up to date for the elapsed time since `last`.
    fn refill(&mut self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        self.available = (self.available + elapsed * self.refill_per_sec).min(self.capacity);
        self.last = now;
    }

    /// The wait until `needed` tokens are available, given the current level.
    fn wait_for(&self, needed: f64) -> Duration {
        if self.available >= needed || self.refill_per_sec <= 0.0 {
            return Duration::ZERO;
        }
        Duration::from_secs_f64((needed - self.available) / self.refill_per_sec)
    }
}

/// Which dimension tripped, for a caller-facing message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LimitKind {
    /// The requests-per-minute limit.
    Requests,
    /// The tokens-per-minute limit.
    Tokens,
}

impl LimitKind {
    /// A short label for the tripped dimension.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Requests => "requests-per-minute",
            Self::Tokens => "tokens-per-minute",
        }
    }
}

/// A rate-limit rejection: which dimension tripped and how long to wait.
#[derive(Clone, Copy, Debug)]
pub struct Rejection {
    /// The dimension that tripped.
    pub kind: LimitKind,
    /// How long the caller should wait before retrying.
    pub retry_after: Duration,
}

impl Rejection {
    /// The `Retry-After` value in whole seconds, always at least 1.
    #[must_use]
    pub fn retry_after_secs(&self) -> u64 {
        self.retry_after.as_secs_f64().ceil().max(1.0) as u64
    }
}

/// Per-key request and token buckets.
#[derive(Debug)]
struct KeyState {
    requests: Option<Bucket>,
    tokens: Option<Bucket>,
}

/// A process-wide, per-key rate limiter.
///
/// Cheap to share behind an [`Arc`](std::sync::Arc); all state is behind one
/// mutex held only for the brief, synchronous bucket arithmetic.
#[derive(Debug, Default)]
pub struct RateLimiter {
    keys: Mutex<HashMap<Uuid, KeyState>>,
}

impl RateLimiter {
    /// A fresh, empty limiter.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Ensures a bucket exists and matches `capacity`, rebuilding it (full) if
    /// the configured capacity changed since it was created.
    fn sync_bucket(slot: &mut Option<Bucket>, capacity: Option<i64>, now: Instant) {
        match capacity {
            Some(cap) if cap > 0 => {
                let cap = cap as f64;
                match slot {
                    Some(b) if (b.capacity - cap).abs() < f64::EPSILON => b.refill(now),
                    _ => *slot = Some(Bucket::new(cap, now)),
                }
            }
            // No limit on this dimension: drop any bucket so it never rejects.
            _ => *slot = None,
        }
    }

    /// Admits one request against `limit`, drawing a request token and checking
    /// the token bucket is not already exhausted.
    ///
    /// Returns `Err(Rejection)` when either dimension is out of budget; the
    /// rejection carries the dimension and the retry-after wait. A `None` (or
    /// fully-unlimited) limit always admits.
    ///
    /// Token *consumption* is deferred to [`record_tokens`](Self::record_tokens)
    /// once the turn's usage is known; this call only rejects when the token
    /// bucket is already empty.
    pub fn check(&self, key_id: Uuid, limit: Option<&RateLimit>) -> Result<(), Rejection> {
        let Some(limit) = limit.filter(|l| l.is_some()) else {
            return Ok(());
        };
        let now = Instant::now();
        let mut keys = self.keys.lock().expect("rate limiter mutex poisoned");
        let state = keys.entry(key_id).or_insert(KeyState {
            requests: None,
            tokens: None,
        });
        Self::sync_bucket(&mut state.requests, limit.requests_per_min, now);
        Self::sync_bucket(&mut state.tokens, limit.tokens_per_min, now);

        // Token bucket: reject if already exhausted (no request without headroom).
        if let Some(tokens) = state.tokens.as_ref() {
            if tokens.available < 1.0 {
                return Err(Rejection {
                    kind: LimitKind::Tokens,
                    retry_after: tokens.wait_for(1.0),
                });
            }
        }
        // Request bucket: draw one, or reject with the wait for one to refill.
        if let Some(requests) = state.requests.as_mut() {
            if requests.available < 1.0 {
                return Err(Rejection {
                    kind: LimitKind::Requests,
                    retry_after: requests.wait_for(1.0),
                });
            }
            requests.available -= 1.0;
        }
        Ok(())
    }

    /// Debits `tokens` from a key's token bucket once a turn's usage is known.
    ///
    /// Best effort: if the key has no token limit this is a no-op. The bucket is
    /// allowed to go to zero (never negative) so a single large turn cannot bank
    /// negative headroom indefinitely; the next request is rejected until it
    /// refills.
    pub fn record_tokens(&self, key_id: Uuid, tokens: u64) {
        if tokens == 0 {
            return;
        }
        let now = Instant::now();
        let mut keys = self.keys.lock().expect("rate limiter mutex poisoned");
        if let Some(state) = keys.get_mut(&key_id) {
            if let Some(bucket) = state.tokens.as_mut() {
                bucket.refill(now);
                bucket.available = (bucket.available - tokens as f64).max(0.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limit(requests: Option<i64>, tokens: Option<i64>) -> RateLimit {
        RateLimit {
            requests_per_min: requests,
            tokens_per_min: tokens,
        }
    }

    #[test]
    fn no_limit_always_admits() {
        let rl = RateLimiter::new();
        let key = Uuid::new_v4();
        for _ in 0..1000 {
            assert!(rl.check(key, None).is_ok());
        }
    }

    #[test]
    fn requests_bucket_rejects_when_drained() {
        let rl = RateLimiter::new();
        let key = Uuid::new_v4();
        let lim = limit(Some(3), None);
        // Three requests admitted, the fourth rejected within the same minute.
        for _ in 0..3 {
            assert!(rl.check(key, Some(&lim)).is_ok());
        }
        let err = rl.check(key, Some(&lim)).expect_err("fourth rejected");
        assert_eq!(err.kind, LimitKind::Requests);
        assert!(err.retry_after_secs() >= 1);
    }

    #[test]
    fn separate_keys_have_separate_buckets() {
        let rl = RateLimiter::new();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let lim = limit(Some(1), None);
        assert!(rl.check(a, Some(&lim)).is_ok());
        assert!(rl.check(a, Some(&lim)).is_err());
        // b is untouched.
        assert!(rl.check(b, Some(&lim)).is_ok());
    }

    #[test]
    fn tokens_bucket_rejects_once_exhausted() {
        let rl = RateLimiter::new();
        let key = Uuid::new_v4();
        let lim = limit(None, Some(100));
        // First request admits (bucket full), then a big turn drains it.
        assert!(rl.check(key, Some(&lim)).is_ok());
        rl.record_tokens(key, 100);
        let err = rl.check(key, Some(&lim)).expect_err("token bucket empty");
        assert_eq!(err.kind, LimitKind::Tokens);
    }

    #[test]
    fn raising_capacity_rebuilds_bucket() {
        let rl = RateLimiter::new();
        let key = Uuid::new_v4();
        let small = limit(Some(1), None);
        assert!(rl.check(key, Some(&small)).is_ok());
        assert!(rl.check(key, Some(&small)).is_err());
        // A new, larger limit rebuilds the bucket full.
        let large = limit(Some(10), None);
        assert!(rl.check(key, Some(&large)).is_ok());
    }
}
