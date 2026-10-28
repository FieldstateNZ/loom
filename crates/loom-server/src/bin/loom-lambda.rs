//! AWS custom runtime entry point. Package as bootstrap for provided.al2023.
#![forbid(unsafe_code)]

use loom_server::{
    lambda::{LambdaGateway, LambdaPolicy},
    Config,
};
use std::{convert::Infallible, process::ExitCode};

#[tokio::main]
async fn main() -> ExitCode {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let gateway = Config::from_env().ok().and_then(|config| {
        LambdaPolicy::from_env()
            .ok()
            .and_then(|policy| LambdaGateway::new(&config, policy).ok())
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
