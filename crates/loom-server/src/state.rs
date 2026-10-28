//! Shared application state handed to every request handler.

use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};

use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use uuid::Uuid;

use loom_provider::Provider;
use loom_store::PgStore;

use crate::batch::{BatchBackend, BatchBackendFactory, DefaultBatchBackendFactory};
use crate::budget::BudgetCache;
use crate::config::Config;
use crate::crypto::Crypto;
use crate::error::ApiError;
use crate::keys::KeyHasher;
use crate::provider::{DefaultProviderFactory, ProviderFactory};
use crate::rate_limit::RateLimiter;
use crate::telemetry::Metrics;
use crate::usage::{OutboxUsageRecorder, UsageRecorder};

/// The state shared across all handlers.
///
/// Cheap to clone: everything lives behind a single [`Arc`], so cloning it (as
/// axum does per request) only bumps a reference count.
#[derive(Clone)]
pub struct AppState {
    inner: Arc<Inner>,
}

struct Inner {
    store: PgStore,
    /// Hold at most half the work pool in long-lived conversation transactions.
    conversation_slots: Arc<Semaphore>,
    cleanup_tasks: Arc<AtomicUsize>,
    cleanup_failed: Arc<AtomicBool>,
    crypto: Crypto,
    hasher: KeyHasher,
    root_admin_token: String,
    factory: Arc<dyn ProviderFactory>,
    batch_factory: Arc<dyn BatchBackendFactory>,
    usage_recorder: Arc<dyn UsageRecorder>,
    /// Per-key, in-process request/token rate limiter (per replica).
    rate_limiter: Arc<RateLimiter>,
    /// Short-TTL, in-process cache of current-window spend (per replica).
    budget_cache: Arc<BudgetCache>,
    /// OpenTelemetry metric instruments (no-ops when no meter is installed).
    metrics: Arc<Metrics>,
}

impl AppState {
    /// Assembles application state from its parts.
    #[must_use]
    pub fn new(
        store: PgStore,
        crypto: Crypto,
        hasher: KeyHasher,
        root_admin_token: String,
    ) -> Self {
        let conversation_slots = Arc::new(Semaphore::new(
            (store.pool().options().get_max_connections() / 2) as usize,
        ));
        Self {
            inner: Arc::new(Inner {
                store,
                conversation_slots,
                cleanup_tasks: Arc::new(AtomicUsize::new(0)),
                cleanup_failed: Arc::new(AtomicBool::new(false)),
                crypto,
                hasher,
                root_admin_token,
                factory: Arc::new(DefaultProviderFactory),
                batch_factory: Arc::new(DefaultBatchBackendFactory),
                usage_recorder: Arc::new(OutboxUsageRecorder),
                rate_limiter: Arc::new(RateLimiter::new()),
                budget_cache: Arc::new(BudgetCache::new()),
                metrics: Arc::new(Metrics::new()),
            }),
        }
    }

    /// Returns a clone of this state with its [`ProviderFactory`] replaced.
    ///
    /// The default state resolves the compiled-in providers via
    /// [`DefaultProviderFactory`]; tests use this to substitute a factory that
    /// returns a mock provider, exercising the conversation API without a live
    /// backend.
    #[must_use]
    pub fn with_provider_factory(self, factory: Arc<dyn ProviderFactory>) -> Self {
        Self {
            inner: Arc::new(Inner {
                store: self.inner.store.clone(),
                conversation_slots: self.inner.conversation_slots.clone(),
                cleanup_tasks: self.inner.cleanup_tasks.clone(),
                cleanup_failed: self.inner.cleanup_failed.clone(),
                crypto: self.inner.crypto.clone(),
                hasher: self.inner.hasher.clone(),
                root_admin_token: self.inner.root_admin_token.clone(),
                factory,
                batch_factory: self.inner.batch_factory.clone(),
                usage_recorder: self.inner.usage_recorder.clone(),
                rate_limiter: self.inner.rate_limiter.clone(),
                budget_cache: self.inner.budget_cache.clone(),
                metrics: self.inner.metrics.clone(),
            }),
        }
    }

    /// Returns a clone of this state with its [`UsageRecorder`] replaced.
    ///
    /// The default records to `usage_events` with an outbox fallback; tests use
    /// this to substitute a recorder that forces the failure path, exercising
    /// the outbox and drain without a real database fault.
    #[must_use]
    pub fn with_usage_recorder(self, usage_recorder: Arc<dyn UsageRecorder>) -> Self {
        Self {
            inner: Arc::new(Inner {
                store: self.inner.store.clone(),
                conversation_slots: self.inner.conversation_slots.clone(),
                cleanup_tasks: self.inner.cleanup_tasks.clone(),
                cleanup_failed: self.inner.cleanup_failed.clone(),
                crypto: self.inner.crypto.clone(),
                hasher: self.inner.hasher.clone(),
                root_admin_token: self.inner.root_admin_token.clone(),
                factory: self.inner.factory.clone(),
                batch_factory: self.inner.batch_factory.clone(),
                usage_recorder,
                rate_limiter: self.inner.rate_limiter.clone(),
                budget_cache: self.inner.budget_cache.clone(),
                metrics: self.inner.metrics.clone(),
            }),
        }
    }

    /// Returns a clone of this state with its [`BatchBackendFactory`] replaced.
    ///
    /// The default resolves an Anthropic-backed batch surface from the tenant's
    /// credential; tests substitute a deterministic fake backend to drive the
    /// batch lifecycle without a live API.
    #[must_use]
    pub fn with_batch_backend_factory(self, batch_factory: Arc<dyn BatchBackendFactory>) -> Self {
        Self {
            inner: Arc::new(Inner {
                store: self.inner.store.clone(),
                conversation_slots: self.inner.conversation_slots.clone(),
                cleanup_tasks: self.inner.cleanup_tasks.clone(),
                cleanup_failed: self.inner.cleanup_failed.clone(),
                crypto: self.inner.crypto.clone(),
                hasher: self.inner.hasher.clone(),
                root_admin_token: self.inner.root_admin_token.clone(),
                factory: self.inner.factory.clone(),
                batch_factory,
                usage_recorder: self.inner.usage_recorder.clone(),
                rate_limiter: self.inner.rate_limiter.clone(),
                budget_cache: self.inner.budget_cache.clone(),
                metrics: self.inner.metrics.clone(),
            }),
        }
    }

    /// Assembles application state from a validated [`Config`] and a connected
    /// store, consuming the config's secrets.
    #[must_use]
    pub fn from_config(config: &Config, store: PgStore) -> Self {
        Self::new(
            store,
            Crypto::new(config.encryption_key()),
            KeyHasher::new(config.key_pepper().to_vec()),
            config.root_admin_token().to_owned(),
        )
    }

    /// The persistence layer.
    #[must_use]
    pub fn store(&self) -> &PgStore {
        &self.inner.store
    }

    /// Admit before borrowing a connection for a long-lived advisory lock.
    /// All state/factory clones share this gate; the remaining pool connections
    /// stay available for lookups, persistence, usage and readiness.
    pub(crate) fn admit_conversation(&self) -> Result<OwnedSemaphorePermit, ApiError> {
        if self.inner.store.pool().options().get_max_connections() < 2 {
            return Err(ApiError::unavailable(
                "conversation execution requires at least two database connections",
            ));
        }
        self.inner
            .conversation_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| {
                ApiError::unavailable(
                    "conversation capacity is busy; try again when an active turn finishes",
                )
            })
    }

    /// Whether this process has released all request-owned database work.
    /// Only meaningful with the request-driven pool policy. This is a local
    /// lifecycle barrier, not a distributed admission or spending limit.
    #[must_use]
    pub fn request_work_idle(&self) -> bool {
        !self.inner.cleanup_failed.load(Ordering::SeqCst)
            && self.inner.cleanup_tasks.load(Ordering::SeqCst) == 0
            && self.inner.conversation_slots.available_permits()
                == (self.store().pool().options().get_max_connections() / 2) as usize
            && self.store().pool().size() == 0
    }

    pub(crate) fn track_cleanup(&self) -> CleanupGuard {
        self.inner.cleanup_tasks.fetch_add(1, Ordering::SeqCst);
        CleanupGuard {
            tasks: self.inner.cleanup_tasks.clone(),
            failed: self.inner.cleanup_failed.clone(),
            complete: false,
        }
    }

    /// The credential encryptor.
    #[must_use]
    pub fn crypto(&self) -> &Crypto {
        &self.inner.crypto
    }

    /// The virtual-key hasher.
    #[must_use]
    pub fn hasher(&self) -> &KeyHasher {
        &self.inner.hasher
    }

    /// The best-effort usage recorder.
    #[must_use]
    pub fn usage_recorder(&self) -> &Arc<dyn UsageRecorder> {
        &self.inner.usage_recorder
    }

    /// The batch-backend factory the poll worker resolves per `(tenant,
    /// provider)`.
    #[must_use]
    pub fn batch_backend_factory(&self) -> &Arc<dyn BatchBackendFactory> {
        &self.inner.batch_factory
    }

    /// Resolves the [`BatchBackend`] bound to `provider` for `tenant_id`.
    ///
    /// # Errors
    ///
    /// Propagates the factory's structured [`ApiError`] when the provider is
    /// unknown, unconfigured, or its credential cannot be decrypted.
    pub async fn resolve_batch_backend(
        &self,
        tenant_id: Uuid,
        provider: &str,
    ) -> Result<Arc<dyn BatchBackend>, ApiError> {
        self.inner
            .batch_factory
            .backend(self, tenant_id, provider)
            .await
    }

    /// The per-key request/token rate limiter (in-process, per replica).
    #[must_use]
    pub fn rate_limiter(&self) -> &RateLimiter {
        &self.inner.rate_limiter
    }

    /// The short-TTL current-window spend cache (in-process, per replica).
    #[must_use]
    pub fn budget_cache(&self) -> &BudgetCache {
        &self.inner.budget_cache
    }

    /// The OpenTelemetry metric instruments (no-ops when no meter is installed).
    #[must_use]
    pub fn metrics(&self) -> &Metrics {
        &self.inner.metrics
    }

    /// Resolves the [`Provider`] bound to `provider` for `tenant_id` via the
    /// configured [`ProviderFactory`].
    ///
    /// # Errors
    ///
    /// Propagates the factory's structured [`ApiError`] when the provider is
    /// unknown, unconfigured, or its credential cannot be decrypted.
    pub async fn resolve_provider(
        &self,
        tenant_id: Uuid,
        provider: &str,
    ) -> Result<Arc<dyn Provider>, ApiError> {
        self.inner.factory.provider(self, tenant_id, provider).await
    }

    /// Constant-time comparison of a presented admin token against the
    /// configured root token.
    ///
    /// Both sides are SHA-256 digested first, so the comparison runs over
    /// fixed-length inputs and leaks neither the token length nor an early
    /// mismatch position.
    #[must_use]
    pub fn verify_admin_token(&self, presented: &str) -> bool {
        let presented = Sha256::digest(presented.as_bytes());
        let expected = Sha256::digest(self.inner.root_admin_token.as_bytes());
        presented.ct_eq(&expected).into()
    }
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("store", &self.inner.store)
            .field("crypto", &self.inner.crypto)
            .field("hasher", &self.inner.hasher)
            .field("root_admin_token", &"<redacted>")
            .finish()
    }
}

/// Registered synchronously before spawning, so a not-yet-polled cleanup task
/// cannot be mistaken for an idle invocation. Aborted/panicking tasks poison
/// quiescence; a Lambda process must then retire instead of freezing.
pub(crate) struct CleanupGuard {
    tasks: Arc<AtomicUsize>,
    failed: Arc<AtomicBool>,
    complete: bool,
}
impl CleanupGuard {
    pub(crate) fn complete(mut self) {
        self.complete = true;
    }
}
impl Drop for CleanupGuard {
    fn drop(&mut self) {
        if !self.complete {
            self.failed.store(true, Ordering::SeqCst);
        }
        self.tasks.fetch_sub(1, Ordering::SeqCst);
    }
}
