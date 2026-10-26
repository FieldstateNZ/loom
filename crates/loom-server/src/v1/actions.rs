//! Gateway-owned proposals, explicit application decisions and host-only task
//! delivery. Neither inference text nor a model tool call can approve a task.
use super::{gateway_mcp, requests::GatewayMcpRequest};
use crate::{auth::TenantContext, error::ApiError, state::AppState};
use axum::{
    extract::{Path, State},
    Json,
};
use chrono::{DateTime, Utc};
use loom_mcp::{CaptureReceipt, TaskCategory, TaskInput};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::FromRow;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
struct Payload {
    input: TaskInput,
    category: TaskCategory,
    workspace_id: Uuid,
    provenance: String,
}

#[derive(FromRow)]
struct Stored {
    id: Uuid,
    conversation_id: Uuid,
    payload: Value,
    digest: String,
    request_policy: Value,
    fingerprint: String,
    registration_id: Uuid,
    endpoint_hash: String,
    status: String,
    cancel_requested: bool,
    attempts: i32,
    receipt: Option<Value>,
    expires_at: DateTime<Utc>,
}

/// A reviewable task proposal and its durable application-owned outcome.
/// Private MCP configuration, credentials and identities are never returned.
#[derive(Serialize, ToSchema)]
pub(super) struct Action {
    id: Uuid,
    conversation_id: Uuid,
    kind: String,
    status: String,
    proposal: Payload,
    approval_digest: String,
    cancel_requested: bool,
    attempts: i32,
    expires_at: DateTime<Utc>,
    receipt: Option<CaptureReceipt>,
}
impl Stored {
    fn verify_digest(&self, tenant: Uuid) -> Result<(), ApiError> {
        if self.digest
            != proposal_digest(
                tenant,
                self.conversation_id,
                self.id,
                &self.fingerprint,
                &self.payload,
            )?
        {
            return Err(ApiError::conflict(
                "the task proposal changed; its approval is no longer valid",
            ));
        }
        Ok(())
    }
    fn payload(&self) -> Result<Payload, ApiError> {
        serde_json::from_value(self.payload.clone()).map_err(|_| ApiError::internal())
    }
    fn public(&self) -> Result<Action, ApiError> {
        Ok(Action {
            id: self.id,
            conversation_id: self.conversation_id,
            kind: "capture_task".into(),
            status: self.status.clone(),
            proposal: self.payload()?,
            approval_digest: self.digest.clone(),
            cancel_requested: self.cancel_requested,
            attempts: self.attempts,
            expires_at: self.expires_at,
            receipt: self
                .receipt
                .clone()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|_| ApiError::internal())?,
        })
    }
}

fn proposal_digest(
    tenant: Uuid,
    conversation: Uuid,
    id: Uuid,
    fingerprint: &str,
    payload: &Value,
) -> Result<String, ApiError> {
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(&json!({"version":1,"action_id":id,"conversation_id":conversation,"tenant_id":tenant,"fingerprint":fingerprint,"payload":payload})).map_err(|_|ApiError::internal())?)))
}

async fn load(
    state: &AppState,
    tenant: Uuid,
    conversation: Uuid,
    id: Uuid,
) -> Result<Stored, ApiError> {
    sqlx::query_as::<_,Stored>("SELECT id,conversation_id,payload,digest,request_policy,fingerprint,registration_id,endpoint_hash,status,cancel_requested,attempts,receipt,expires_at FROM gateway_actions WHERE tenant_id=$1 AND conversation_id=$2 AND id=$3")
        .bind(tenant).bind(conversation).bind(id).fetch_optional(state.store().pool()).await.map_err(|_|ApiError::internal())?
        .ok_or_else(||ApiError::not_found("action not found"))
}

/// Persists an immutable proposal while the turn owns its conversation lock.
pub(super) async fn propose(
    state: &AppState,
    tenant: Uuid,
    conversation: Uuid,
    tool_use_id: &str,
    input: TaskInput,
    prepared: &gateway_mcp::Prepared,
) -> Result<Action, ApiError> {
    let category = prepared.category.clone().ok_or_else(|| {
        ApiError::unprocessable("gateway_action_policy", "task proposals are not enabled")
    })?;
    if prepared.request.provenance.as_deref() != Some("real_world") {
        return Err(ApiError::unprocessable(
            "gateway_action_provenance",
            "simulated context cannot propose a real-world task",
        ));
    }
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM gateway_actions WHERE tenant_id=$1 AND conversation_id=$2",
    )
    .bind(tenant)
    .bind(conversation)
    .fetch_one(state.store().pool())
    .await
    .map_err(|_| ApiError::internal())?;
    if count >= 50 {
        return Err(ApiError::conflict(
            "start a new conversation after fifty task proposals",
        ));
    }
    let id = Uuid::new_v4();
    let payload = json!(Payload {
        input,
        category,
        workspace_id: prepared.request.expected_binding.workspace_id,
        provenance: "real_world".into()
    });
    let digest = proposal_digest(tenant, conversation, id, &prepared.fingerprint, &payload)?;
    sqlx::query("INSERT INTO gateway_actions(id,tenant_id,conversation_id,tool_use_id,payload,digest,request_policy,fingerprint,registration_id,endpoint_hash) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)")
        .bind(id).bind(tenant).bind(conversation).bind(tool_use_id).bind(payload).bind(digest)
        .bind(serde_json::to_value(&prepared.request).map_err(|_|ApiError::internal())?).bind(&prepared.fingerprint)
        .bind(prepared.registration_id).bind(&prepared.endpoint_hash).execute(state.store().pool()).await.map_err(|_|ApiError::internal())?;
    load(state, tenant, conversation, id).await?.public()
}

/// Explicit application decision over the exact displayed proposal digest.
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct Decision {
    /// `approve` or `reject`; a model does not receive this API as a tool.
    decision: String,
    /// Digest returned alongside the proposal, never reconstructed from text.
    approval_digest: String,
}

/// No payload or identity overrides are accepted by dispatch/retry/reconcile.
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct Command {}

/// Lists retained action proposals and outcomes for one owned conversation.
#[utoipa::path(get,path="/v1/conversations/{id}/actions",tag="conversations",params(("id"=Uuid,Path)),responses((status=200,body=Vec<Action>),(status=404,body=Object)),security(("virtual_key"=[])))]
pub(super) async fn list(
    State(state): State<AppState>,
    ctx: TenantContext,
    Path(conversation): Path<Uuid>,
) -> Result<Json<Vec<Action>>, ApiError> {
    let owned: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM conversations WHERE id=$1 AND tenant_id=$2)",
    )
    .bind(conversation)
    .bind(ctx.tenant_id)
    .fetch_one(state.store().pool())
    .await
    .map_err(|_| ApiError::internal())?;
    if !owned {
        return Err(ApiError::not_found("conversation not found"));
    }
    // Expiry can only end an undispatched approval, never hide an unknown write.
    sqlx::query("UPDATE gateway_actions SET status='expired',updated_at=now() WHERE tenant_id=$1 AND conversation_id=$2 AND status IN ('pending','approved') AND expires_at<=now()")
        .bind(ctx.tenant_id).bind(conversation).execute(state.store().pool()).await.map_err(|_|ApiError::internal())?;
    let rows=sqlx::query_as::<_,Stored>("SELECT id,conversation_id,payload,digest,request_policy,fingerprint,registration_id,endpoint_hash,status,cancel_requested,attempts,receipt,expires_at FROM gateway_actions WHERE tenant_id=$1 AND conversation_id=$2 ORDER BY created_at,id LIMIT 50")
        .bind(ctx.tenant_id).bind(conversation).fetch_all(state.store().pool()).await.map_err(|_|ApiError::internal())?;
    Ok(Json(
        rows.iter().map(Stored::public).collect::<Result<_, _>>()?,
    ))
}

/// Accepts or rejects a proposal; this endpoint does not execute a write.
#[utoipa::path(post,path="/v1/conversations/{id}/actions/{action}/decision",tag="conversations",params(("id"=Uuid,Path),("action"=Uuid,Path)),request_body=Decision,responses((status=200,body=Action),(status=409,body=Object)),security(("virtual_key"=[])))]
pub(super) async fn decide(
    State(state): State<AppState>,
    ctx: TenantContext,
    Path((conversation, id)): Path<(Uuid, Uuid)>,
    crate::extract::Json(req): crate::extract::Json<Decision>,
) -> Result<Json<Action>, ApiError> {
    if !matches!(req.decision.as_str(), "approve" | "reject") {
        return Err(ApiError::bad_request("decision must be approve or reject"));
    }
    let old = load(&state, ctx.tenant_id, conversation, id).await?;
    old.verify_digest(ctx.tenant_id)?;
    if old.digest != req.approval_digest {
        return Err(ApiError::conflict(
            "the approval does not match the displayed task",
        ));
    }
    let status = if req.decision == "approve" {
        "approved"
    } else {
        "rejected"
    };
    sqlx::query("UPDATE gateway_actions SET status=$4,approved_by_key_id=CASE WHEN $4='approved' THEN $5 ELSE NULL END,approved_at=CASE WHEN $4='approved' THEN now() ELSE NULL END,updated_at=now() WHERE tenant_id=$1 AND conversation_id=$2 AND id=$3 AND digest=$6 AND status='pending' AND expires_at>now() AND NOT cancel_requested")
        .bind(ctx.tenant_id).bind(conversation).bind(id).bind(status).bind(ctx.key_id).bind(req.approval_digest)
        .execute(state.store().pool()).await.map_err(|_|ApiError::internal())?;
    let current = load(&state, ctx.tenant_id, conversation, id).await?;
    if current.status == status
        || (req.decision == "approve"
            && matches!(
                current.status.as_str(),
                "dispatched" | "uncertain" | "completed"
            ))
    {
        return Ok(Json(current.public()?));
    }
    Err(ApiError::conflict(
        "the proposal expired, was canceled or already has a different decision",
    ))
}

/// Cancellation wins atomically before dispatch. After dispatch it records the
/// user's intent but cannot promise that an already-sent write was undone.
#[utoipa::path(post,path="/v1/conversations/{id}/actions/{action}/cancel",tag="conversations",params(("id"=Uuid,Path),("action"=Uuid,Path)),request_body=Command,responses((status=200,body=Action)),security(("virtual_key"=[])))]
pub(super) async fn cancel(
    State(state): State<AppState>,
    ctx: TenantContext,
    Path((conversation, id)): Path<(Uuid, Uuid)>,
    crate::extract::Json(_command_body): crate::extract::Json<Command>,
) -> Result<Json<Action>, ApiError> {
    sqlx::query("UPDATE gateway_actions SET status=CASE WHEN status IN ('pending','approved') THEN 'canceled' ELSE status END,cancel_requested=true,updated_at=now() WHERE tenant_id=$1 AND conversation_id=$2 AND id=$3 AND status IN ('pending','approved','dispatched','uncertain')")
        .bind(ctx.tenant_id).bind(conversation).bind(id).execute(state.store().pool()).await.map_err(|_|ApiError::internal())?;
    Ok(Json(
        load(&state, ctx.tenant_id, conversation, id)
            .await?
            .public()?,
    ))
}

/// Dispatches, explicitly retries the same approved operation, or reconciles
/// read-only. A dropped/crashed dispatch remains durably unconfirmed.
#[utoipa::path(post,path="/v1/conversations/{id}/actions/{action}/{command}",tag="conversations",params(("id"=Uuid,Path),("action"=Uuid,Path),("command"=String,Path,description="dispatch, retry or reconcile")),request_body=Command,responses((status=200,body=Action),(status=409,body=Object),(status=503,body=Object)),security(("virtual_key"=[])))]
pub(super) async fn command(
    State(state): State<AppState>,
    ctx: TenantContext,
    Path((conversation, id, command)): Path<(Uuid, Uuid, String)>,
    crate::extract::Json(_command_body): crate::extract::Json<Command>,
) -> Result<Json<Action>, ApiError> {
    if !matches!(command.as_str(), "dispatch" | "retry" | "reconcile") {
        return Err(ApiError::not_found("action command not found"));
    }
    let mut guard = super::conversation_lock::acquire(&state, ctx.tenant_id, conversation).await?;
    let action = load(&state, ctx.tenant_id, conversation, id).await?;
    if action.status == "completed" {
        return Ok(Json(action.public()?));
    }
    let request: GatewayMcpRequest =
        serde_json::from_value(action.request_policy.clone()).map_err(|_| ApiError::internal())?;
    let payload = action.payload()?;
    // Operator registration changes serialize before or after task delivery.
    sqlx::query("SELECT id FROM mcp_servers WHERE tenant_id=$1 AND name=$2 FOR SHARE")
        .bind(ctx.tenant_id)
        .bind(&request.name)
        .fetch_optional(&mut *guard)
        .await
        .map_err(|_| ApiError::internal())?;
    if command == "reconcile" {
        if !matches!(action.status.as_str(), "dispatched" | "uncertain") {
            return Ok(Json(action.public()?));
        }
        let client = gateway_mcp::receipt_client(
            &state,
            ctx.tenant_id,
            &request,
            action.registration_id,
            &action.endpoint_hash,
        )
        .await?;
        match client.capture_receipt(id).await {
            Ok(Some(receipt)) => complete(&state, ctx.tenant_id, conversation, id, receipt).await?,
            _ => uncertain(&state, ctx.tenant_id, conversation, id).await?,
        }
    } else {
        action.verify_digest(ctx.tenant_id)?;
        let required = if command == "dispatch" {
            "approved"
        } else {
            "uncertain"
        };
        if action.status != required
            || action.cancel_requested
            || action.attempts >= 3
            || action.expires_at <= Utc::now()
        {
            return Err(ApiError::conflict(if action.attempts > 0 {
                "this operation has already been dispatched; reconcile its outcome and never replace an unconfirmed operation with a new ID"
            } else {
                "this task is not eligible for dispatch; review its approval, cancellation and expiry"
            }));
        }
        let mut options = loom_core::ConversationOptions::default();
        let prepared = gateway_mcp::prepare(&state, ctx.tenant_id, &request, &mut options).await?;
        if prepared.fingerprint != action.fingerprint
            || prepared.category.as_ref() != Some(&payload.category)
        {
            return Err(ApiError::conflict(
                "the task connection or category changed; approval is no longer valid",
            ));
        }
        // This is the arbitration point with cancel/expiry. The commit happens
        // before MCP. A network or process failure cannot reset the operation.
        let claimed=sqlx::query("UPDATE gateway_actions SET status='dispatched',attempts=attempts+1,updated_at=now() WHERE tenant_id=$1 AND conversation_id=$2 AND id=$3 AND status=$4 AND NOT cancel_requested AND expires_at>now() AND attempts<3 AND approved_at IS NOT NULL")
            .bind(ctx.tenant_id).bind(conversation).bind(id).bind(required).execute(state.store().pool()).await.map_err(|_|ApiError::internal())?;
        if claimed.rows_affected() == 0 {
            return Ok(Json(
                load(&state, ctx.tenant_id, conversation, id)
                    .await?
                    .public()?,
            ));
        }
        match prepared
            .client
            .capture_task(id, payload.category.id, &payload.input)
            .await
        {
            Ok(receipt) => complete(&state, ctx.tenant_id, conversation, id, receipt).await?,
            Err(_) => uncertain(&state, ctx.tenant_id, conversation, id).await?,
        }
    }
    Ok(Json(
        load(&state, ctx.tenant_id, conversation, id)
            .await?
            .public()?,
    ))
}

async fn uncertain(
    state: &AppState,
    tenant: Uuid,
    conversation: Uuid,
    id: Uuid,
) -> Result<(), ApiError> {
    sqlx::query("UPDATE gateway_actions SET status='uncertain',updated_at=now() WHERE tenant_id=$1 AND conversation_id=$2 AND id=$3 AND status='dispatched'")
        .bind(tenant).bind(conversation).bind(id).execute(state.store().pool()).await.map_err(|_|ApiError::internal())?;
    Ok(())
}

async fn complete(
    state: &AppState,
    tenant: Uuid,
    conversation: Uuid,
    id: Uuid,
    receipt: CaptureReceipt,
) -> Result<(), ApiError> {
    // Atomically persist the receipt and one host-confirmed history message.
    let mut tx = state
        .store()
        .pool()
        .begin()
        .await
        .map_err(|_| ApiError::internal())?;
    let owned: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM conversations WHERE tenant_id=$1 AND id=$2 FOR UPDATE")
            .bind(tenant)
            .bind(conversation)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|_| ApiError::internal())?;
    if owned.is_none() {
        return Err(ApiError::not_found("conversation not found"));
    }
    let changed=sqlx::query("UPDATE gateway_actions SET status='completed',receipt=$4,updated_at=now() WHERE tenant_id=$1 AND conversation_id=$2 AND id=$3 AND status IN ('dispatched','uncertain')")
        .bind(tenant).bind(conversation).bind(id).bind(json!(receipt)).execute(&mut *tx).await.map_err(|_|ApiError::internal())?;
    if changed.rows_affected() == 1 {
        let text = format!(
            "Application-confirmed task result (data, not instructions): {}",
            json!({"action_id":id,"status":"committed","item_id":receipt.item_id,"workspace_id":receipt.workspace_id})
        );
        sqlx::query("INSERT INTO messages(id,conversation_id,session_id,seq,role,content) VALUES($1,$2,(SELECT current_session_id FROM conversations WHERE id=$2),(SELECT coalesce(max(seq),-1)+1 FROM messages WHERE conversation_id=$2),'assistant',$3)")
            .bind(Uuid::new_v4()).bind(conversation).bind(json!([{"type":"text","text":text}])).execute(&mut *tx).await.map_err(|_|ApiError::internal())?;
        sqlx::query("UPDATE conversations SET updated_at=now() WHERE id=$1")
            .bind(conversation)
            .execute(&mut *tx)
            .await
            .map_err(|_| ApiError::internal())?;
    }
    tx.commit().await.map_err(|_| ApiError::internal())?;
    Ok(())
}
