//! Shared test setup for the `loom-store` integration test binaries.
//!
//! Each `tests/*.rs` file is compiled as its own binary, so this module (kept
//! under `tests/common/`, not `tests/*.rs`, so cargo does not treat it as a
//! test binary itself) is `mod`-included by every file that needs a live,
//! migrated database.
//!
//! Two ways to obtain that database:
//!
//! - **Default:** a throwaway PostgreSQL 16 container via `testcontainers`
//!   (needs a Docker daemon). This is the long-standing path and stays the
//!   default so local runs and the main CI `test` job are unchanged.
//! - **External server:** when `LOOM_TEST_DATABASE_URL` is set, no container
//!   is started. The harness connects to that server's maintenance database,
//!   creates a fresh, uniquely named database for the test, migrates it and
//!   hands back a store scoped to it. CI uses this to run the same migration
//!   and store tests against other engines/images (e.g. PostgreSQL 18, the
//!   CloudNativePG candidate image) started outside the test process. Tests
//!   never share a database, so they remain independent and order-agnostic.
//!   The per-test databases are left behind on purpose: the server is
//!   ephemeral and dropping them would race in-flight pools.

use std::str::FromStr;

use loom_store::{run_migrations, PgStore};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use testcontainers::runners::AsyncRunner;
use testcontainers::ContainerAsync;
use testcontainers::ImageExt;
use testcontainers_modules::postgres::Postgres;

/// Environment variable naming an external PostgreSQL server to test against.
pub const EXTERNAL_DATABASE_URL_ENV: &str = "LOOM_TEST_DATABASE_URL";

/// Keeps the test database alive for the duration of a test.
///
/// For the container path this owns the running container (dropping it stops
/// PostgreSQL); for the external path it is a marker. Bind it to `_pg` in the
/// test so it is dropped at the end of the test, not immediately.
pub struct TestDatabase {
    _container: Option<ContainerAsync<Postgres>>,
}

/// Boots a fresh, migrated database and returns a guard (which must be kept
/// alive for the duration of the test) plus a connected store.
pub async fn setup() -> (TestDatabase, PgStore) {
    setup_database(true).await
}

/// Isolated empty database for upgrade-chain tests; honours the same external
/// engine selection as setup(). Other test binaries include this module too.
#[allow(dead_code)]
pub async fn setup_unmigrated() -> (TestDatabase, PgStore) {
    setup_database(false).await
}

async fn setup_database(migrate: bool) -> (TestDatabase, PgStore) {
    match std::env::var(EXTERNAL_DATABASE_URL_ENV) {
        Ok(url) if !url.trim().is_empty() => setup_external(url.trim(), migrate).await,
        _ => setup_container(migrate).await,
    }
}

async fn setup_container(migrate: bool) -> (TestDatabase, PgStore) {
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
    if migrate {
        run_migrations(store.pool())
            .await
            .expect("migrations apply cleanly from empty database");
    }
    (
        TestDatabase {
            _container: Some(container),
        },
        store,
    )
}

async fn setup_external(url: &str, migrate: bool) -> (TestDatabase, PgStore) {
    let base = PgConnectOptions::from_str(url)
        .unwrap_or_else(|e| panic!("{EXTERNAL_DATABASE_URL_ENV} is not a valid URL: {e}"));

    // One short-lived maintenance connection to create the per-test database.
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(base.clone())
        .await
        .unwrap_or_else(|e| panic!("connect to {EXTERNAL_DATABASE_URL_ENV}: {e}"));
    let db_name = format!("loomtest_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE \"{db_name}\""))
        .execute(&admin)
        .await
        .expect("create per-test database");
    admin.close().await;

    let pool = PgPoolOptions::new()
        .connect_with(base.database(&db_name))
        .await
        .expect("connect to per-test database");
    let store = PgStore::new(pool);
    if migrate {
        run_migrations(store.pool())
            .await
            .expect("migrations apply cleanly from empty database");
    }
    (TestDatabase { _container: None }, store)
}
