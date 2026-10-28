//! Real PostgreSQL: no inference, no cloud and no timing-only mock of releases.
use loom_server::database::runtime_pool_options;
use sqlx::{postgres::PgPoolOptions, PgPool};
use std::time::Duration;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;

async fn idle(pool: &PgPool, observer: &PgPool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let sessions: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity WHERE application_name='request-driven-test'",
            ).fetch_one(observer).await.unwrap();
            if pool.size() == 0 && sessions == 0 { break; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("released connections must really close on the database");
}

#[tokio::test]
async fn request_driven_pool_reconnects_and_preserves_transaction_cleanup() {
    let container = Postgres::default().start().await.unwrap();
    let url = format!(
        "postgres://postgres:postgres@127.0.0.1:{}/postgres",
        container.get_host_port_ipv4(5432).await.unwrap()
    );
    let observer = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE lifecycle_receipts (id integer PRIMARY KEY)")
        .execute(&observer)
        .await
        .unwrap();
    let pool = runtime_pool_options(2, true)
        .connect(&format!("{url}?application_name=request-driven-test"))
        .await
        .unwrap();
    let first: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&pool)
        .await
        .unwrap();
    idle(&pool, &observer).await;
    let second: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_ne!(first, second);
    idle(&pool, &observer).await;

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(632419)")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO lifecycle_receipts VALUES (1)")
        .execute(&mut *tx)
        .await
        .unwrap();
    drop(tx); // A canceled caller must not strand a lock or commit its receipt.
    idle(&pool, &observer).await;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM lifecycle_receipts")
        .fetch_one(&observer)
        .await
        .unwrap();
    assert_eq!(count, 0);
    let lock_free: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(632419)")
        .fetch_one(&observer)
        .await
        .unwrap();
    assert!(lock_free);

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("INSERT INTO lifecycle_receipts VALUES (1)")
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    idle(&pool, &observer).await;
    assert!(sqlx::query("SELECT 1/0").execute(&pool).await.is_err());
    idle(&pool, &observer).await;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM lifecycle_receipts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    idle(&pool, &observer).await;

    // The ordinary server profile still reuses healthy idle connections.
    let warm = runtime_pool_options(2, false).connect(&url).await.unwrap();
    let first: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&warm)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    let second: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&warm)
        .await
        .unwrap();
    assert_eq!(first, second);
    warm.close().await;
    pool.close().await;
    observer.close().await;
}
