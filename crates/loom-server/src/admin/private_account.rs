//! Operator-only initial account creation. The operator durably proposes the
//! key before sending it; retries recover IDs without storing a decryptable key.
use crate::{error::ApiError, extract, state::AppState};
use axum::{
    extract::{Path, State},
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use http::{header::CACHE_CONTROL, StatusCode};
use loom_store::{NewPrivateAccount, PrivateAccountStore};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

// Intentionally not Debug: this request contains a proposed virtual-key secret.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProvisionRequest {
    schema_version: u8,
    issuer: String,
    subject: String,
    operation_id: Uuid,
    key: String,
}

fn validated(
    req: &ProvisionRequest,
    namespace: &str,
    state: &AppState,
) -> Result<NewPrivateAccount, ApiError> {
    let invalid = || ApiError::bad_request("invalid private account request");
    let issuer = reqwest_mcp::Url::parse(&req.issuer).map_err(|_| invalid())?;
    if req.schema_version != 1
        || req.operation_id.get_version_num() != 4
        || req.issuer.len() > 512
        || issuer.scheme() != "https"
        || !issuer.username().is_empty()
        || issuer.password().is_some()
        || issuer.query().is_some()
        || issuer.fragment().is_some()
        || issuer.as_str() != req.issuer
        || req.subject.is_empty()
        || req.subject.len() > 2048
        || req.subject.trim() != req.subject
        || req.subject.chars().any(char::is_control)
    {
        return Err(invalid());
    }
    let owner = serde_json::to_vec(&[&req.issuer, &req.subject]).map_err(|_| invalid())?;
    let expected = format!("tandem-{}", &hex::encode(Sha256::digest(owner))[..40]);
    if expected != namespace {
        return Err(invalid());
    }
    let (label, body) = if let Some(body) = req.key.strip_prefix("loom_live_") {
        ("live", body)
    } else if let Some(body) = req.key.strip_prefix("loom_test_") {
        ("test", body)
    } else {
        return Err(invalid());
    };
    let bytes = URL_SAFE_NO_PAD.decode(body).map_err(|_| invalid())?;
    if bytes.len() != 32 || URL_SAFE_NO_PAD.encode(&bytes) != body {
        return Err(invalid());
    }
    let key_hash = state.hasher().hash(&req.key);
    let request_hash = hex::encode(Sha256::digest(
        serde_json::to_vec(&(1, &req.issuer, &req.subject, req.operation_id, &key_hash))
            .map_err(|_| invalid())?,
    ));
    Ok(NewPrivateAccount {
        namespace: expected,
        operation_id: req.operation_id,
        request_hash,
        key_hash,
        key_prefix: format!("loom_{label}_{}", &body[..6]),
    })
}

pub(super) async fn provision(
    State(state): State<AppState>,
    Path(namespace): Path<String>,
    extract::Json(req): extract::Json<ProvisionRequest>,
) -> Result<impl axum::response::IntoResponse, ApiError> {
    let new = validated(&req, &namespace, &state)?;
    let prefix = new.key_prefix.clone();
    let result=state.store().provision_private_account(new).await.map_err(ApiError::from_store)?
        .ok_or_else(||ApiError::conflict("private account setup cannot be created or replayed; preserve the original private request"))?;
    Ok((
        if result.created {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        [(CACHE_CONTROL, "no-store")],
        Json(
            serde_json::json!({"schema_version":1,"state":"active","namespace":result.namespace,"operation_id":result.operation_id,"tenant_id":result.tenant_id,"key_id":result.key_id,"key_prefix":prefix,"created_at":result.created_at}),
        ),
    ))
}
