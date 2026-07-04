//! The default [`BatchBackendFactory`] and its Anthropic-backed
//! [`BatchBackend`] implementation.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use uuid::Uuid;

use loom_provider::ProviderError;
use loom_provider_anthropic::{
    translate, AnthropicProvider, BatchRequest, PROVIDER_NAME as ANTHROPIC_PROVIDER,
};
use loom_store::BatchItemStatus;

use crate::error::ApiError;
use crate::state::AppState;

use super::backend::{BatchBackend, BatchBackendFactory};
use super::snapshot::{ProviderBatchResult, ProviderBatchSnapshot};
use super::submit_item::BatchSubmitItem;

/// The default factory over the providers compiled into the gateway. Recognises
/// `"anthropic"`; any other name is a `422`.
#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultBatchBackendFactory;

#[async_trait]
impl BatchBackendFactory for DefaultBatchBackendFactory {
    async fn backend(
        &self,
        state: &AppState,
        tenant_id: Uuid,
        provider: &str,
    ) -> Result<Arc<dyn BatchBackend>, ApiError> {
        match provider {
            ANTHROPIC_PROVIDER => {
                let credential =
                    crate::provider::load_credential(state, tenant_id, provider).await?;
                let api_key = crate::provider::decrypt_api_key(state, &credential)?;
                let mut anthropic =
                    AnthropicProvider::new(api_key).map_err(ApiError::from_provider)?;
                if let Some(base_url) = credential.base_url {
                    anthropic = anthropic.with_base_url(base_url);
                }
                Ok(Arc::new(AnthropicBatchBackend {
                    provider: anthropic,
                }))
            }
            other => Err(ApiError::unprocessable(
                "unknown_provider",
                format!("provider {other:?} does not support batches on this gateway"),
            )),
        }
    }
}

/// The Anthropic-backed [`BatchBackend`]: translates each item to a native
/// Messages request and drives Anthropic's Message Batches API.
struct AnthropicBatchBackend {
    provider: AnthropicProvider,
}

impl AnthropicBatchBackend {
    /// Maps a native Anthropic batch snapshot to the provider-agnostic shape.
    fn snapshot(batch: &loom_provider_anthropic::AnthropicBatch) -> ProviderBatchSnapshot {
        let clamp = |v: i64| i32::try_from(v.max(0)).unwrap_or(i32::MAX);
        ProviderBatchSnapshot {
            provider_batch_id: batch.id.clone(),
            ended: batch.is_ended(),
            counts: loom_store::BatchCounts {
                processing: clamp(batch.counts.processing),
                succeeded: clamp(batch.counts.succeeded),
                errored: clamp(batch.counts.errored),
                canceled: clamp(batch.counts.canceled),
                expired: clamp(batch.counts.expired),
            },
            results_url: batch.results_url.clone(),
        }
    }
}

#[async_trait]
impl BatchBackend for AnthropicBatchBackend {
    async fn submit(
        &self,
        items: Vec<BatchSubmitItem>,
    ) -> Result<ProviderBatchSnapshot, ProviderError> {
        let requests: Vec<BatchRequest> = items
            .iter()
            .map(|item| BatchRequest {
                custom_id: item.custom_id.clone(),
                params: translate::translate_request(&item.conversation, &item.options),
            })
            .collect();
        let batch = self.provider.create_batch(&requests).await?;
        Ok(Self::snapshot(&batch))
    }

    async fn poll(&self, provider_batch_id: &str) -> Result<ProviderBatchSnapshot, ProviderError> {
        let batch = self.provider.get_batch(provider_batch_id).await?;
        Ok(Self::snapshot(&batch))
    }

    async fn results(
        &self,
        snapshot: &ProviderBatchSnapshot,
    ) -> Result<Vec<ProviderBatchResult>, ProviderError> {
        let Some(url) = snapshot.results_url.as_deref() else {
            return Ok(Vec::new());
        };
        let raw = self.provider.fetch_batch_results(url).await?;
        Ok(raw
            .into_iter()
            .map(|r| {
                let outcome = match r.result.get("type").and_then(Value::as_str) {
                    Some("succeeded") => BatchItemStatus::Succeeded,
                    Some("canceled") => BatchItemStatus::Canceled,
                    Some("expired") => BatchItemStatus::Expired,
                    // Anything else (including "errored") is a failure.
                    _ => BatchItemStatus::Errored,
                };
                let usage = if outcome == BatchItemStatus::Succeeded {
                    r.result
                        .get("message")
                        .and_then(|m| m.get("usage"))
                        .map(translate::translate_usage)
                } else {
                    None
                };
                ProviderBatchResult {
                    custom_id: r.custom_id,
                    outcome,
                    result: r.result,
                    usage,
                }
            })
            .collect())
    }

    async fn cancel(
        &self,
        provider_batch_id: &str,
    ) -> Result<ProviderBatchSnapshot, ProviderError> {
        let batch = self.provider.cancel_batch(provider_batch_id).await?;
        Ok(Self::snapshot(&batch))
    }
}
