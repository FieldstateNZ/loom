//! Operator readiness uses the same registered-credential connection as a turn,
//! without creating a conversation, invoking inference or dispatching a task.
use super::{prepare, Registry};
use crate::{error::ApiError, state::AppState, v1::requests::GatewayMcpRequest};
use loom_core::ConversationOptions;
use loom_mcp::Binding;
use loom_store::{McpServerStore, TenantStore};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreflightRequest {
    pub(crate) expected_binding: Binding,
}

pub(crate) async fn preflight(
    state: &AppState,
    tenant: Uuid,
    name: &str,
    input: PreflightRequest,
) -> Result<Value, ApiError> {
    if name != "lucidbrain" {
        return Err(ApiError::not_found("private memory server not supported"));
    }
    tokio::time::timeout(std::time::Duration::from_secs(15),async {
        let owner=state.store().get_tenant(tenant).await.map_err(ApiError::from_store)?
            .ok_or_else(||ApiError::not_found("tenant not found"))?;
        if owner.status!="active" {return Err(ApiError::unavailable("private memory owner is unavailable"));}
        let before=state.store().get_mcp_server(tenant,name).await.map_err(ApiError::from_store)?
            .ok_or_else(||ApiError::not_found("private memory registration not found"))?;
        let request=GatewayMcpRequest{name:name.to_owned(),allowed_tools:vec!["memory_search".to_owned()],expected_binding:input.expected_binding.clone(),propose_tasks:true,provenance:Some("real_world".to_owned())};
        let prepared=prepare(state,tenant,&request,&mut ConversationOptions::default()).await?;
        // Hold the actual client's private authorization lease (where enabled)
        // while checking the snapshot. Concurrent static registration changes
        // fail rather than attesting a different policy/token than we used.
        let after=state.store().get_mcp_server(tenant,name).await.map_err(ApiError::from_store)?;
        let owner=state.store().get_tenant(tenant).await.map_err(ApiError::from_store)?;
        if after.as_ref()!=Some(&before)||owner.is_none_or(|t|t.status!="active") {
            return Err(ApiError::unavailable("private memory registration changed during preflight"));
        }
        let mut policy:Registry=serde_json::from_value(before.tool_configuration.unwrap_or_default()).map_err(|_|ApiError::internal())?;
        policy.allowed_tools.sort();policy.allowed_tools.dedup();
        let category=prepared.category.as_ref().ok_or_else(ApiError::internal)?;
        Ok(json!({"schema_version":1,"credential_preflight_version":1,"state":"ready","tenant_id":tenant,"registration_id":prepared.registration_id,"name":name,
            "binding":input.expected_binding,"allowed_tools":policy.allowed_tools,"category_id":category.id,"transport_mode":"stateless"}))
    }).await.map_err(|_|ApiError::unavailable("private memory preflight timed out"))?
}
