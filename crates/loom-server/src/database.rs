//! Database lifecycle policy shared by the executable and integration tests.

use sqlx::postgres::PgPoolOptions;
use std::time::Duration;

/// Build the runtime pool with a finite connection budget. Request-driven mode
/// discards each connection when its checkout ends, including transaction locks.
/// This removes idle sockets without relying on timers running during a freeze.
/// Release cleanup is asynchronous in SQLx: an invocation adapter still needs a
/// quiescence barrier before freezing; this policy alone is not Lambda proof.
#[must_use]
pub fn runtime_pool_options(max_connections: u32, request_driven: bool) -> PgPoolOptions {
    PgPoolOptions::new()
        .max_connections(max_connections)
        .min_connections(0)
        .acquire_timeout(Duration::from_secs(10))
        .after_release(move |_connection, _metadata| Box::pin(async move { Ok(!request_driven) }))
}
