//! AWS custom runtime entry point; use the matching packaged container runtime.
#![forbid(unsafe_code)]

use loom_server::{
    lambda::{LambdaGateway, LambdaPolicy},
    lambda_bootstrap,
};
use std::{convert::Infallible, process::ExitCode};

#[tokio::main]
async fn main() -> ExitCode {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let policy = LambdaPolicy::from_env().ok();
    let loaded = if policy.is_some() {
        lambda_bootstrap::load_config().await.ok()
    } else {
        None
    };
    let gateway = loaded.as_ref().and_then(|loaded| {
        policy.and_then(|policy| LambdaGateway::new(loaded.config(), policy).ok())
    });
    let Some(gateway) = gateway else {
        // Configuration errors can contain DSNs/secrets; do not print them.
        eprintln!("Private Lambda configuration is invalid; gateway not started.");
        return ExitCode::FAILURE;
    };
    let result = lambda_http::run(lambda_http::service_fn(move |request| {
        let gateway = gateway.clone();
        async move { Ok::<_, Infallible>(gateway.handle(request).await) }
    }))
    .await;
    if result.is_ok() {
        ExitCode::SUCCESS
    } else {
        eprintln!("Private Lambda runtime stopped.");
        ExitCode::FAILURE
    }
}
