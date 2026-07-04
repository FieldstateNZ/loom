//! Turning provider-batch outcomes into stored state: building the submission
//! from stored items, and finalising an ended job with per-item results and
//! priced usage.

use chrono::Utc;
use uuid::Uuid;

use loom_core::Usage;
use loom_store::{
    BatchItem, BatchItemStatus, BatchJob, BatchStatus, BatchStore, NewUsageEvent, Pricer,
    PricingStore,
};

use crate::state::AppState;

use super::backend::BatchBackend;
use super::conversation_from;
use super::dto::BatchItemInput;
use super::snapshot::ProviderBatchSnapshot;
use super::submit_item::BatchSubmitItem;

/// Builds the provider submission from stored items, reconstructing each turn.
pub(super) fn build_submit_items(
    tenant_id: Uuid,
    items: &[BatchItem],
) -> Result<Vec<BatchSubmitItem>, String> {
    items
        .iter()
        .map(|item| {
            let input: BatchItemInput = serde_json::from_value(item.request.clone())
                .map_err(|e| format!("decode stored item {}: {e}", item.custom_id))?;
            let (conversation, options) = conversation_from(tenant_id, &input);
            Ok(BatchSubmitItem {
                custom_id: item.custom_id.clone(),
                conversation,
                options,
            })
        })
        .collect()
}

/// Finalises an ended job: fetches results, persists each item's outcome,
/// records batch-tier usage for the succeeded items, and marks the job ended.
pub(super) async fn finalize_job(
    state: &AppState,
    job: &BatchJob,
    backend: &dyn BatchBackend,
    snapshot: &ProviderBatchSnapshot,
) -> Result<(), String> {
    let results = backend
        .results(snapshot)
        .await
        .map_err(|e| format!("fetch results: {e}"))?;

    // Map custom_id → model so batch usage is priced against the right model.
    let items = state
        .store()
        .get_batch_items(job.id)
        .await
        .map_err(|e| format!("load items: {e}"))?;
    let model_for = |custom_id: &str| -> Option<String> {
        items
            .iter()
            .find(|i| i.custom_id == custom_id)
            .map(|i| i.model.clone())
    };

    for result in &results {
        state
            .store()
            .save_batch_item_result(
                job.tenant_id,
                job.id,
                &result.custom_id,
                result.outcome,
                &result.result,
            )
            .await
            .map_err(|e| format!("save item result: {e}"))?;

        if let (BatchItemStatus::Succeeded, Some(usage), Some(model)) = (
            result.outcome,
            result.usage.clone(),
            model_for(&result.custom_id),
        ) {
            record_batch_usage(state, job, &model, usage).await;
        }
    }

    state
        .store()
        .update_batch_progress(
            job.tenant_id,
            job.id,
            BatchStatus::Ended,
            snapshot.counts,
            snapshot.results_url.as_deref(),
            Some(Utc::now()),
        )
        .await
        .map_err(|e| format!("finalize: {e}"))?;
    Ok(())
}

/// Records a priced, batch-tier usage event for one succeeded item (best
/// effort, via the state's outbox-backed recorder).
async fn record_batch_usage(state: &AppState, job: &BatchJob, model: &str, usage: Usage) {
    let cost = match state
        .store()
        .get_effective_price(&job.provider, model, Utc::now())
        .await
    {
        Ok(Some(price)) => Some(Pricer::cost_with_mode(&usage, &price, true)),
        Ok(None) => None,
        Err(err) => {
            tracing::warn!(error = %err, "batch price lookup failed; recording usage without cost");
            None
        }
    };
    state.metrics().record_tokens(
        job.tenant_id,
        model,
        usage.input_tokens.unwrap_or(0),
        usage.output_tokens.unwrap_or(0),
    );
    if let Some(cost) = cost {
        state.metrics().record_cost(job.tenant_id, model, cost);
    }
    let event = NewUsageEvent {
        tenant_id: job.tenant_id,
        virtual_key_id: job.virtual_key_id,
        conversation_id: None,
        provider: job.provider.clone(),
        model: model.to_owned(),
        usage,
        cost,
        is_batch: true,
    };
    state.usage_recorder().record(state.store(), event).await;
}
