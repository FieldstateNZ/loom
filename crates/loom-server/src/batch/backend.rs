//! The provider batch seam: [`BatchBackend`] (submit/poll/fetch/cancel a
//! provider batch) and [`BatchBackendFactory`] (resolves a backend for a
//! tenant/provider pair).

use std::sync::Arc;

use async_trait::async_trait;
use uuid::Uuid;

use loom_provider::ProviderError;

use crate::error::ApiError;
use crate::state::AppState;

use super::snapshot::{ProviderBatchResult, ProviderBatchSnapshot};
use super::submit_item::BatchSubmitItem;

/// A provider's batch surface: submit, poll, fetch results, cancel.
///
/// The seam the [poll worker](super::poll::run_batch_poll_pass) drives. The
/// production implementation wraps
/// [`AnthropicProvider`](loom_provider_anthropic::AnthropicProvider)'s batch
/// methods; tests inject a deterministic fake so the whole lifecycle runs
/// without a live API or real time.
#[async_trait]
pub trait BatchBackend: Send + Sync {
    /// Submits `items` as a new provider batch.
    async fn submit(
        &self,
        items: Vec<BatchSubmitItem>,
    ) -> Result<ProviderBatchSnapshot, ProviderError>;

    /// Polls the current state of the provider batch `provider_batch_id`.
    async fn poll(&self, provider_batch_id: &str) -> Result<ProviderBatchSnapshot, ProviderError>;

    /// Retrieves the per-item results for an ended `snapshot`.
    async fn results(
        &self,
        snapshot: &ProviderBatchSnapshot,
    ) -> Result<Vec<ProviderBatchResult>, ProviderError>;

    /// Requests cancellation of the provider batch `provider_batch_id`,
    /// returning the updated snapshot.
    async fn cancel(&self, provider_batch_id: &str)
        -> Result<ProviderBatchSnapshot, ProviderError>;
}

/// Resolves a [`BatchBackend`] for a `(tenant, provider)` pair.
///
/// Mirrors [`ProviderFactory`](crate::provider::ProviderFactory): credential
/// loading and decryption happen here so the worker never touches secrets.
#[async_trait]
pub trait BatchBackendFactory: Send + Sync {
    /// Resolves `provider` for `tenant_id`, returning a shared backend handle.
    async fn backend(
        &self,
        state: &AppState,
        tenant_id: Uuid,
        provider: &str,
    ) -> Result<Arc<dyn BatchBackend>, ApiError>;
}
