//! The background poll worker: [`spawn_batch_worker`].

use std::time::Duration;

use crate::state::AppState;

use super::poll::run_batch_poll_pass;

/// Spawns the background poll worker, advancing active batches every `interval`.
///
/// A no-op forever-pending task when `interval` is zero (worker disabled). The
/// returned handle is detached by the caller; the worker exits when the process
/// does.
#[must_use]
pub fn spawn_batch_worker(state: AppState, interval: Duration) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if interval.is_zero() {
            return;
        }
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            let report = run_batch_poll_pass(&state).await;
            if report.advanced > 0 || report.errored > 0 {
                tracing::debug!(
                    advanced = report.advanced,
                    errored = report.errored,
                    "batch poll pass complete"
                );
            }
        }
    })
}
