//! Shared test setup for the `loom-store` integration test binaries.
//!
//! Each `tests/*.rs` file is compiled as its own binary, so this module (kept
//! under `tests/common/`, not `tests/*.rs`, so cargo does not treat it as a
//! test binary itself) is `mod`-included by every file that needs a live,
//! migrated database.

use loom_store::{run_migrations, PgStore};
use testcontainers::runners::AsyncRunner;
use testcontainers::ContainerAsync;
use testcontainers::ImageExt;
use testcontainers_modules::postgres::Postgres;

/// Boots a fresh, migrated database and returns the live container (which must
/// be kept alive for the duration of the test) plus a connected store.
pub async fn setup() -> (ContainerAsync<Postgres>, PgStore) {
    let container = Postgres::default()
        .with_tag("16")
        .start()
        .await
        .expect("start postgres container");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("map postgres port");
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let store = PgStore::connect(&url).await.expect("connect to postgres");
    run_migrations(store.pool())
        .await
        .expect("migrations apply cleanly from empty database");
    (container, store)
}
