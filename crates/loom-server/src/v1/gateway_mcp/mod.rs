//! Opt-in Loom-owned memory execution. Provider APIs receive function schemas,
//! never MCP URLs, bearer credentials or the identity preflight payload.
use super::requests::GatewayMcpRequest;
use crate::{error::ApiError, state::AppState};
use loom_core::ConversationOptions;
use loom_mcp::{Binding, MemoryClient};
use loom_store::McpServerStore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

mod collect;
mod execute;
pub(super) use execute::execute;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Registry {
    enabled: bool,
    allowed_tools: Vec<String>,
    gateway: Gateway,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Gateway {
    kind: String,
    binding: Binding,
}

pub(super) struct Prepared {
    client: MemoryClient,
    fingerprint: String,
}

pub(super) async fn runtime(
    state: &AppState,
    tenant: Uuid,
    id: Uuid,
) -> Result<Option<(String, String)>, ApiError> {
    sqlx::query_as("SELECT fingerprint,status FROM gateway_mcp_runtime WHERE tenant_id=$1 AND conversation_id=$2")
        .bind(tenant).bind(id).fetch_optional(state.store().pool()).await.map_err(|_|ApiError::internal())
}

pub(super) async fn prepare(
    state: &AppState,
    tenant: Uuid,
    request: &GatewayMcpRequest,
    options: &mut ConversationOptions,
) -> Result<Prepared, ApiError> {
    if request.name != "lucidbrain"
        || request.allowed_tools.is_empty()
        || request.allowed_tools.len() > 2
        || !options.tools.is_empty()
        || !options.mcp_servers.is_empty()
        || !options.server_tools.is_empty()
        || !options.provider_options.is_empty()
        || options.max_tokens.is_some_and(|n| n > 2000)
    {
        return Err(ApiError::unprocessable("gateway_mcp_policy", "private memory requires an explicit registered read-tool policy without mixed tool executors"));
    }
    options.max_tokens.get_or_insert(700);
    let registered = state
        .store()
        .get_mcp_server(tenant, &request.name)
        .await
        .map_err(ApiError::from_store)?
        .ok_or_else(|| {
            ApiError::unprocessable(
                "mcp_server_not_configured",
                "private memory server is not registered",
            )
        })?;
    let mut policy: Registry = serde_json::from_value(
        registered.tool_configuration.clone().unwrap_or_default(),
    )
    .map_err(|_| {
        ApiError::unprocessable(
            "gateway_mcp_policy",
            "private memory registration is incomplete",
        )
    })?;
    if !policy.enabled
        || policy.gateway.kind != "lucidbrain_memory"
        || policy.allowed_tools.is_empty()
        || policy.allowed_tools.len() > 2
        || policy
            .allowed_tools
            .iter()
            .any(|name| !loom_mcp::is_memory_read(name))
    {
        return Err(ApiError::unprocessable(
            "gateway_mcp_policy",
            "private memory registration is not enabled for reviewed reads",
        ));
    }
    policy.allowed_tools.sort();
    policy.allowed_tools.dedup();
    if policy.gateway.binding != request.expected_binding {
        return Err(ApiError::unavailable(
            "private memory registration does not match the application's provisioned identity",
        ));
    }
    let mut requested = request.allowed_tools.clone();
    requested.sort();
    requested.dedup();
    let token = crate::provider::decrypt_mcp_token(state, tenant, &registered.name, &registered)?
        .ok_or_else(|| {
        ApiError::unprocessable(
            "gateway_mcp_policy",
            "private memory requires an assistant credential",
        )
    })?;
    let client = MemoryClient::connect(
        &registered.url,
        &token,
        policy.gateway.binding.clone(),
        &policy.allowed_tools,
        &requested,
    )
    .await
    .map_err(|_| {
        ApiError::unavailable(
            "private memory identity, permission or connection verification failed",
        )
    })?;
    options.tools = client.tools().to_vec();
    let fingerprint = hex::encode(Sha256::digest(
        serde_json::to_vec(&(registered.id, &registered.url, policy, requested))
            .map_err(|_| ApiError::internal())?,
    ));
    Ok(Prepared {
        client,
        fingerprint,
    })
}

pub(super) async fn pin(
    state: &AppState,
    tenant: Uuid,
    id: Uuid,
    prepared: &Prepared,
) -> Result<(), ApiError> {
    let prior = runtime(state, tenant, id).await?;
    if let Some((fingerprint, status)) = prior {
        if fingerprint != prepared.fingerprint || status != "idle" {
            return Err(ApiError::conflict("private memory binding changed or an earlier run needs reconciliation; start a new conversation"));
        }
    } else {
        sqlx::query("INSERT INTO gateway_mcp_runtime(tenant_id,conversation_id,fingerprint) VALUES($1,$2,$3)")
            .bind(tenant).bind(id).bind(&prepared.fingerprint).execute(state.store().pool()).await.map_err(|_|ApiError::internal())?;
    }
    Ok(())
}

pub(super) async fn mark(
    state: &AppState,
    tenant: Uuid,
    id: Uuid,
    status: &str,
) -> Result<(), ApiError> {
    let result=sqlx::query("UPDATE gateway_mcp_runtime SET status=$3,updated_at=now() WHERE tenant_id=$1 AND conversation_id=$2")
        .bind(tenant).bind(id).bind(status).execute(state.store().pool()).await.map_err(|_|ApiError::internal())?;
    if result.rows_affected() != 1 {
        return Err(ApiError::conflict(
            "private memory conversation is unavailable",
        ));
    }
    Ok(())
}
