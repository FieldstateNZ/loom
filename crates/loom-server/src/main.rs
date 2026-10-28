//! `loom-server` binary entry point.
//!
//! Loads [`Config`] from the environment, connects the PostgreSQL store,
//! optionally applies migrations, and serves [`build_router`]. All HTTP surface
//! lives in the [`loom_server`] library.
#![forbid(unsafe_code)]

use std::process::ExitCode;

use loom_server::config::DEFAULT_BIND_ADDR;
use loom_server::{build_router, telemetry, AppState, Config};
use loom_store::PgStore;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> ExitCode {
    // SQLx can establish TLS before the MCP client is constructed. Choose the
    // workspace's ring backend here when transitive dependencies also enable
    // aws-lc-rs; otherwise Rustls cannot choose a default and panics.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mode = match mode(std::env::args().skip(1).collect()) {
        Ok(mode) => mode,
        Err(()) => {
            eprintln!("Usage: loom-server [--migrate | --healthcheck | --help]");
            return ExitCode::from(2);
        }
    };
    match mode {
        Mode::Healthcheck => return ExitCode::from(healthcheck()),
        Mode::Help => {
            println!("Usage: loom-server [--migrate | --healthcheck | --help]\n--migrate reads DATABASE_URL, applies/verifies schema, and exits without starting HTTP, workers or provider clients.");
            return ExitCode::SUCCESS;
        }
        Mode::Migrate => return migrate_only().await,
        Mode::Serve => {}
    }

    // Installs JSON logs and, when an OTLP endpoint is configured, the
    // OpenTelemetry trace + metric exporters. The guard flushes them on drop, so
    // it must outlive `run()`.
    let _telemetry = telemetry::init();

    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!(error = %err, "loom-server failed to start");
            ExitCode::FAILURE
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Mode {
    Serve,
    Migrate,
    Healthcheck,
    Help,
}
fn mode(args: Vec<String>) -> Result<Mode, ()> {
    match args.as_slice() {
        [] => Ok(Mode::Serve),
        [arg] => match arg.as_str() {
            "--migrate" => Ok(Mode::Migrate),
            "--healthcheck" => Ok(Mode::Healthcheck),
            "--help" => Ok(Mode::Help),
            _ => Err(()),
        },
        _ => Err(()),
    }
}

// Deliberately bypass normal config/telemetry: migration jobs need only the
// database credential, not admin/encryption/provider secrets or network clients.
async fn migrate_only() -> ExitCode {
    let Some(url) = std::env::var("DATABASE_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
    else {
        eprintln!("Migration requires DATABASE_URL.");
        return ExitCode::FAILURE;
    };
    let timeout = match std::env::var("LOOM_MIGRATION_TIMEOUT_SECS") {
        Ok(v) => match v.parse::<u64>().ok().filter(|n| (1..=900).contains(n)) {
            Some(n) => n,
            None => {
                eprintln!("LOOM_MIGRATION_TIMEOUT_SECS must be from 1 to 900.");
                return ExitCode::FAILURE;
            }
        },
        Err(std::env::VarError::NotPresent) => 120,
        Err(_) => {
            eprintln!("Invalid migration timeout.");
            return ExitCode::FAILURE;
        }
    };
    let result = tokio::time::timeout(std::time::Duration::from_secs(timeout), async {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(std::time::Duration::from_secs(10))
            .connect(&url)
            .await?;
        let result = async {
            loom_store::run_migrations(&pool).await?;
            loom_store::verify_migrations(&pool).await
        }
        .await;
        pool.close().await;
        result
    })
    .await;
    if matches!(result, Ok(Ok::<(), loom_store::StoreError>(()))) {
        println!("Database migrations completed and verified.");
        ExitCode::SUCCESS
    } else {
        // DSNs can carry passwords, CA paths and private hostnames. Keep CLI
        // failures non-diagnostic; operators inspect the private DB job context.
        eprintln!("Database migration failed or timed out; release was not started.");
        ExitCode::FAILURE
    }
}

/// Boots and serves the gateway, returning an error rather than panicking so the
/// process exits with a clean, logged failure.
async fn run() -> anyhow::Result<()> {
    let config = Config::from_env()?;

    let pool = loom_server::database::runtime_pool_options(
        config.database_max_connections,
        config.request_driven,
    )
    .connect(&config.database_url)
    .await?;
    let store = PgStore::new(pool);

    if config.run_migrations {
        tracing::info!("applying database migrations");
        loom_store::run_migrations(store.pool()).await?;
    }

    // Schema verification is SELECT-only. A dedicated migration job must
    // finish before a runtime with no DDL privileges can accept any traffic.
    loom_store::verify_migrations(store.pool()).await?;

    let bind_addr = config.bind_addr;
    let state = AppState::from_config(&config, store);

    // Advance asynchronous batch jobs on a background interval (disabled when the
    // interval is zero, e.g. when a dedicated worker owns it).
    if !config.batch_poll_interval.is_zero() {
        tracing::info!(
            interval_secs = config.batch_poll_interval.as_secs(),
            "starting batch poll worker"
        );
        let _worker = loom_server::spawn_batch_worker(state.clone(), config.batch_poll_interval);
    }

    let app = build_router(state);

    let listener = TcpListener::bind(bind_addr).await?;
    tracing::info!(%bind_addr, version = loom_core::VERSION, "loom-server listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

/// Waits for either Ctrl-C or SIGTERM (sent by `docker stop` / Kubernetes).
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
    tracing::info!("shutdown signal received");
}

/// Dependency-free liveness probe used by the container HEALTHCHECK.
///
/// Connects to the locally bound port, issues `GET /healthz`, and returns a
/// process exit code (`0` healthy, `1` unhealthy) based on the status line.
fn healthcheck() -> u8 {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    let bind_addr =
        std::env::var("LOOM_BIND_ADDR").unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_string());
    let port = bind_addr
        .rsplit(':')
        .next()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(8080);

    let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) else {
        return 1;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    if stream
        .write_all(b"GET /healthz HTTP/1.0\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .is_err()
    {
        return 1;
    }
    let mut response = String::new();
    if stream.read_to_string(&mut response).is_err() {
        return 1;
    }
    match response.lines().next() {
        Some(status_line) if status_line.contains("200") => 0,
        _ => 1,
    }
}

#[cfg(test)]
mod mode_tests {
    #[test]
    fn commands_are_exclusive_and_unknown_flags_cannot_start_a_service() {
        use super::{mode, Mode};
        assert_eq!(mode(vec![]), Ok(Mode::Serve));
        assert_eq!(mode(vec!["--migrate".into()]), Ok(Mode::Migrate));
        assert_eq!(mode(vec!["--healthcheck".into()]), Ok(Mode::Healthcheck));
        assert_eq!(mode(vec!["--help".into()]), Ok(Mode::Help));
        for args in [
            vec!["--migrate", "--healthcheck"],
            vec!["--migrate", "--migrate"],
            vec!["--unknown"],
        ] {
            assert!(mode(args.into_iter().map(str::to_owned).collect()).is_err());
        }
    }
}
