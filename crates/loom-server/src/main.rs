//! `loom-server` — the Loom HTTP gateway.
//!
//! > **Scaffold.** Currently serves only a `GET /healthz` liveness endpoint so
//! > that `docker compose up` yields a healthy container. The `/v1` conversation
//! > API, auth middleware, budgets and usage rollups land across issues #7–#14.
#![forbid(unsafe_code)]

use std::net::SocketAddr;

use axum::{routing::get, Router};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

const DEFAULT_BIND_ADDR: &str = "0.0.0.0:8080";

#[tokio::main]
async fn main() {
    // Docker/compose HEALTHCHECK path: `loom-server --healthcheck` performs a
    // dependency-free HTTP probe of the running server and exits 0/1.
    if std::env::args().any(|arg| arg == "--healthcheck") {
        std::process::exit(healthcheck());
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .json()
        .init();

    let bind_addr =
        std::env::var("LOOM_BIND_ADDR").unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_string());
    let addr: SocketAddr = bind_addr
        .parse()
        .unwrap_or_else(|e| panic!("LOOM_BIND_ADDR must be host:port ({bind_addr:?}): {e}"));

    let app = Router::new()
        .route("/healthz", get(healthz))
        .layer(tower_http::trace::TraceLayer::new_for_http());

    let listener = TcpListener::bind(addr)
        .await
        .unwrap_or_else(|e| panic!("failed to bind {addr}: {e}"));
    tracing::info!(%addr, version = loom_core::VERSION, "loom-server listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server error");
}

/// Liveness endpoint. Returns `ok` with a 200 status.
async fn healthz() -> &'static str {
    "ok"
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
fn healthcheck() -> i32 {
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
