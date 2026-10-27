//! Opt-in Loom-owned memory execution. Provider APIs receive function schemas,
//! never MCP URLs, bearer credentials or the identity preflight payload.
use super::requests::GatewayMcpRequest;
use crate::{error::ApiError, state::AppState};
use loom_core::ConversationOptions;
use loom_mcp::{Binding, MemoryClient, TaskCategory};
use loom_store::McpServerStore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

mod collect;
mod execute;
pub(super) use execute::execute;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Registry {
    enabled: bool,
    allowed_tools: Vec<String>,
    gateway: Gateway,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Gateway {
    kind: String,
    binding: Binding,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    task_capture: Option<TaskPolicy>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TaskPolicy {
    workspace_slug: String,
    category_id: Uuid,
    provenance: String,
}

pub(super) struct Prepared {
    pub(super) client: crate::private_mcp::LeasedClient,
    pub(super) fingerprint: String,
    pub(super) category: Option<TaskCategory>,
    pub(super) request: GatewayMcpRequest,
    pub(super) registration_id: Uuid,
    pub(super) endpoint_hash: String,
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
        || (request.allowed_tools.is_empty() && !request.propose_tasks)
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
        || policy.allowed_tools.len() > if request.propose_tasks { 5 } else { 2 }
        || policy
            .allowed_tools
            .iter()
            .any(|name| !private_tool(name, request.propose_tasks))
    {
        return Err(ApiError::unprocessable(
            "gateway_mcp_policy",
            "private memory registration is not enabled for reviewed reads",
        ));
    }
    if request.propose_tasks
        && (request.provenance.as_deref() != Some("real_world")
            || policy
                .gateway
                .task_capture
                .as_ref()
                .is_none_or(|p| p.provenance != "real_world"))
    {
        return Err(ApiError::unprocessable(
            "gateway_action_policy",
            "task proposals require explicit real-world application and operator provenance",
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
    let (token, lease) = crate::private_mcp::authorization(state, &registered).await?;
    let client = if request.propose_tasks {
        MemoryClient::connect_tasks(
            &registered.url,
            &token,
            policy.gateway.binding.clone(),
            &policy.allowed_tools,
            &requested,
        )
        .await
    } else {
        MemoryClient::connect(
            &registered.url,
            &token,
            policy.gateway.binding.clone(),
            &policy.allowed_tools,
            &requested,
        )
        .await
    }
    .map_err(|_| {
        ApiError::unavailable(
            "private memory identity, permission or connection verification failed",
        )
    })?;
    let category = if request.propose_tasks {
        let target = policy
            .gateway
            .task_capture
            .as_ref()
            .ok_or_else(ApiError::internal)?;
        Some(
            client
                .task_category(&target.workspace_slug, target.category_id)
                .await
                .map_err(|_| {
                    ApiError::unavailable("the approved task category could not be verified")
                })?,
        )
    } else {
        None
    };
    options.tools = client.tools().to_vec();
    let fingerprint = hex::encode(Sha256::digest(
        serde_json::to_vec(&(registered.id, &registered.url, policy, requested))
            .map_err(|_| ApiError::internal())?,
    ));
    Ok(Prepared {
        client: crate::private_mcp::LeasedClient {
            client,
            _lease: lease,
        },
        fingerprint,
        category,
        request: request.clone(),
        registration_id: registered.id,
        endpoint_hash: hex::encode(Sha256::digest(registered.url.as_bytes())),
    })
}

fn private_tool(name: &str, tasks: bool) -> bool {
    loom_mcp::is_memory_read(name)
        || (tasks
            && matches!(
                name,
                "list_categories" | "capture_item" | "get_capture_operation"
            ))
}

/// Reconciliation may follow intentional credential replacement, but cannot
/// change the principal, workspace, assistant, registration or endpoint.
pub(super) async fn receipt_client(
    state: &AppState,
    tenant: Uuid,
    request: &GatewayMcpRequest,
    registration_id: Uuid,
    endpoint_hash: &str,
) -> Result<crate::private_mcp::LeasedClient, ApiError> {
    let server = state
        .store()
        .get_mcp_server(tenant, &request.name)
        .await
        .map_err(ApiError::from_store)?
        .ok_or_else(|| ApiError::unavailable("task connection is unavailable"))?;
    let policy: Registry =
        serde_json::from_value(server.tool_configuration.clone().unwrap_or_default())
            .map_err(|_| ApiError::unavailable("task connection is unavailable"))?;
    let a = &policy.gateway.binding;
    let b = &request.expected_binding;
    if !policy.enabled
        || policy.gateway.kind != "lucidbrain_memory"
        || server.id != registration_id
        || hex::encode(Sha256::digest(server.url.as_bytes())) != endpoint_hash
        || a.principal_id != b.principal_id
        || a.workspace_id != b.workspace_id
        || a.agent_definition_id != b.agent_definition_id
        || policy.allowed_tools.iter().any(|n| !private_tool(n, true))
    {
        return Err(ApiError::unavailable(
            "task reconciliation identity or policy changed",
        ));
    }
    let (token, lease) = crate::private_mcp::authorization(state, &server).await?;
    let client = MemoryClient::connect_receipts(
        &server.url,
        &token,
        policy.gateway.binding,
        &policy.allowed_tools,
    )
    .await
    .map_err(|_| {
        ApiError::unavailable(
            "task reconciliation identity, permission or connection verification failed",
        )
    })?;
    Ok(crate::private_mcp::LeasedClient {
        client,
        _lease: lease,
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
