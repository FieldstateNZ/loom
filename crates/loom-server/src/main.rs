//! `loom-server` binary entry point.
//!
//! Loads [`Config`] from the environment, connects the PostgreSQL store,
//! optionally applies migrations, and serves [`build_router`]. All HTTP surface
//! lives in the [`loom_server`] library.
#![forbid(unsafe_code)]

use std::process::ExitCode;

use loom_server::config::DEFAULT_BIND_ADDR;
use loom_server::{build_router, AppState, Config};
use loom_store::PgStore;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> ExitCode {
    // Docker/compose HEALTHCHECK path: `loom-server --healthcheck` performs a
    // dependency-free HTTP probe of the running server and exits 0/1.
    if std::env::args().any(|arg| arg == "--healthcheck") {
        return ExitCode::from(healthcheck());
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .json()
        .init();

    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!(error = %err, "loom-server failed to start");
            ExitCode::FAILURE
        }
    }
}

/// Boots and serves the gateway, returning an error rather than panicking so the
/// process exits with a clean, logged failure.
async fn run() -> anyhow::Result<()> {
    let config = Config::from_env()?;

    let store = PgStore::connect(&config.database_url).await?;

    if config.run_migrations {
        tracing::info!("applying database migrations");
        loom_store::run_migrations(store.pool()).await?;
    }

    let bind_addr = config.bind_addr;
    let state = AppState::from_config(&config, store);
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
