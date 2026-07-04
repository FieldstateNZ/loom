//! HTTP client tests for the Anthropic Message Batches surface, driven against a
//! `wiremock` server rather than a live API.
//!
//! Covers request shaping and response parsing for create, poll, JSONL results
//! retrieval, and cancel.

use std::time::Duration;

use loom_provider_anthropic::{AnthropicProvider, BatchRequest};
use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn provider(server: &MockServer) -> AnthropicProvider {
    AnthropicProvider::new("test-key")
        .expect("build provider")
        .with_base_url(server.uri())
        .with_max_retries(1)
        .with_retry_base_delay(Duration::from_millis(1))
}

#[tokio::test]
async fn create_poll_results_and_cancel() {
    let server = MockServer::start().await;

    // Create → in_progress.
    Mock::given(method("POST"))
        .and(path("/v1/messages/batches"))
        .and(header("x-api-key", "test-key"))
        .and(header("anthropic-version", "2023-06-01"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "msgbatch_abc",
            "processing_status": "in_progress",
            "request_counts": {
                "processing": 2, "succeeded": 0, "errored": 0,
                "canceled": 0, "expired": 0
            },
            "results_url": null,
            "ended_at": null
        })))
        .mount(&server)
        .await;

    let batch = provider(&server)
        .create_batch(&[
            BatchRequest {
                custom_id: "a".to_owned(),
                params: json!({ "model": "claude-opus-4-8", "messages": [] }),
            },
            BatchRequest {
                custom_id: "b".to_owned(),
                params: json!({ "model": "claude-opus-4-8", "messages": [] }),
            },
        ])
        .await
        .expect("create batch");
    assert_eq!(batch.id, "msgbatch_abc");
    assert!(!batch.is_ended());
    assert_eq!(batch.counts.processing, 2);

    // Poll → ended, with a results URL on the mock server.
    let results_url = format!("{}/v1/messages/batches/msgbatch_abc/results", server.uri());
    Mock::given(method("GET"))
        .and(path("/v1/messages/batches/msgbatch_abc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "msgbatch_abc",
            "processing_status": "ended",
            "request_counts": {
                "processing": 0, "succeeded": 2, "errored": 0,
                "canceled": 0, "expired": 0
            },
            "results_url": results_url,
            "ended_at": "2026-07-04T00:00:00Z"
        })))
        .mount(&server)
        .await;

    let ended = provider(&server)
        .get_batch("msgbatch_abc")
        .await
        .expect("poll batch");
    assert!(ended.is_ended());
    assert_eq!(ended.counts.succeeded, 2);
    let url = ended.results_url.clone().expect("results url");

    // Results — a JSONL document, one result object per line.
    let jsonl = "{\"custom_id\":\"a\",\"result\":{\"type\":\"succeeded\",\"message\":{\"role\":\"assistant\"}}}\n\
                 {\"custom_id\":\"b\",\"result\":{\"type\":\"errored\",\"error\":{\"type\":\"invalid_request\"}}}\n";
    Mock::given(method("GET"))
        .and(path("/v1/messages/batches/msgbatch_abc/results"))
        .respond_with(ResponseTemplate::new(200).set_body_string(jsonl))
        .mount(&server)
        .await;

    let results = provider(&server)
        .fetch_batch_results(&url)
        .await
        .expect("fetch results");
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].custom_id, "a");
    assert_eq!(results[0].result["type"], "succeeded");
    assert_eq!(results[1].custom_id, "b");
    assert_eq!(results[1].result["type"], "errored");

    // Cancel → canceling.
    Mock::given(method("POST"))
        .and(path("/v1/messages/batches/msgbatch_abc/cancel"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "msgbatch_abc",
            "processing_status": "canceling",
            "request_counts": {
                "processing": 2, "succeeded": 0, "errored": 0,
                "canceled": 0, "expired": 0
            },
            "results_url": null,
            "ended_at": null
        })))
        .mount(&server)
        .await;

    let canceling = provider(&server)
        .cancel_batch("msgbatch_abc")
        .await
        .expect("cancel batch");
    assert_eq!(canceling.processing_status, "canceling");
    assert!(!canceling.is_ended());
}
