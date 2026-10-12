//! Records which PostgreSQL engine the store tests ran against and, when CI
//! asks for a specific major version, refuses to pass against any other.
//!
//! The compatibility jobs set `LOOM_TEST_EXPECT_PG_MAJOR` together with
//! `LOOM_TEST_DATABASE_URL`; this guard is what makes "the suite passed on
//! PostgreSQL 18" a checked claim rather than an assumption that the harness
//! did not silently fall back to its default PostgreSQL 16 container.

mod common;

/// Environment variable naming the required PostgreSQL major version.
const EXPECT_MAJOR_ENV: &str = "LOOM_TEST_EXPECT_PG_MAJOR";

#[tokio::test]
async fn reports_engine_and_enforces_expected_major_version() {
    let (_pg, store) = common::setup().await;

    let version: String = sqlx::query_scalar("SELECT version()")
        .fetch_one(store.pool())
        .await
        .expect("SELECT version()");
    let version_num: String = sqlx::query_scalar("SHOW server_version_num")
        .fetch_one(store.pool())
        .await
        .expect("SHOW server_version_num");
    let major = version_num
        .parse::<i64>()
        .expect("server_version_num is an integer")
        / 10_000;
    eprintln!("loom-store tests ran against: {version} (major {major})");

    if let Ok(expected) = std::env::var(EXPECT_MAJOR_ENV) {
        let expected: i64 = expected
            .trim()
            .parse()
            .unwrap_or_else(|e| panic!("{EXPECT_MAJOR_ENV} must be an integer: {e}"));
        assert_eq!(
            major, expected,
            "{EXPECT_MAJOR_ENV}={expected} but the server is PostgreSQL {major}: {version}"
        );
    }
}

#[tokio::test]
async fn migrated_schema_has_every_expected_table() {
    let (_pg, store) = common::setup().await;

    // Tables created by the embedded migration set, in migration order. A
    // migration that silently no-ops on a newer engine would show up here.
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT table_name::text FROM information_schema.tables \
         WHERE table_schema = 'public' AND table_type = 'BASE TABLE' ORDER BY table_name",
    )
    .fetch_all(store.pool())
    .await
    .expect("list tables");
    for expected in [
        "_sqlx_migrations",
        "agent_definition_versions",
        "agent_definitions",
        "batch_items",
        "batch_jobs",
        "conversations",
        "mcp_servers",
        "messages",
        "model_prices",
        "pending_tool_calls",
        "provider_credentials",
        "session_events",
        "sessions",
        "tenants",
        "usage_events",
        "usage_outbox",
        "virtual_keys",
    ] {
        assert!(
            tables.iter().any(|t| t == expected),
            "table {expected} missing after migrations; got {tables:?}"
        );
    }

    // Every embedded migration file must be recorded as applied successfully.
    let embedded = std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/migrations"))
        .expect("read migrations dir")
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "sql"))
        .count() as i64;
    let applied: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE success")
        .fetch_one(store.pool())
        .await
        .expect("count applied migrations");
    assert_eq!(
        applied, embedded,
        "expected all {embedded} embedded migrations to be recorded as applied"
    );
}
