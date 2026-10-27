//! Private token renewal is a pre-dispatch host operation, never a tool retry.
//! The original pair + random operation are committed before the first HTTP
//! request and retained after every unknown outcome. No provider/phone sees it.
use std::{ops::Deref, time::Duration};

use chrono::{DateTime, Utc};
use futures::StreamExt;
use loom_mcp::{Binding, MemoryClient};
use loom_store::{McpCredentialLease, McpServer, McpServerStore};
use reqwest_mcp::{Client, Url};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{error::ApiError, state::AppState};

const HORIZON: i64 = 120;
const GRANT: &str = "urn:fieldstate:params:oauth:grant-type:recoverable-refresh-v1";

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExternalAccount {
    schema_version: u32,
    account_id: Uuid,
    issuer: String,
    subject: String,
    user_id: Uuid,
    default_category_id: Uuid,
}

/// Private admin input. Intentionally no Debug implementation for secrets.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Install {
    client_id: String,
    binding: Binding,
    external_account: ExternalAccount,
    access_token: String,
    refresh_token: String,
    scope: String,
    access_expires_at: DateTime<Utc>,
    refresh_expires_at: DateTime<Utc>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    version: u32,
    registration: String,
    credential: Install,
    pending: Option<Uuid>,
    fenced: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Renewal {
    grant_version: u32,
    operation_id: Uuid,
    client_id: String,
    credential_id: Uuid,
    token_type: String,
    access_token: String,
    refresh_token: String,
    expires_in: i64,
    access_expires_at: DateTime<Utc>,
    refresh_expires_at: DateTime<Utc>,
    recovery_expires_at: DateTime<Utc>,
    scope: String,
    state: String,
    binding: Binding,
    external_account: ExternalAccount,
}

/// Keep the lease alive for exactly the actual MCP client lifetime. Field order
/// cancels the MCP transport before the database connection releases its lock.
pub(crate) struct LeasedClient {
    pub(crate) client: MemoryClient,
    pub(crate) _lease: Option<AuthorizationLease>,
}
impl Deref for LeasedClient {
    type Target = MemoryClient;
    fn deref(&self) -> &Self::Target {
        &self.client
    }
}

pub(crate) struct AuthorizationLease {
    guard: Option<McpCredentialLease>,
    admission: Option<tokio::sync::OwnedSemaphorePermit>,
}
impl Drop for AuthorizationLease {
    fn drop(&mut self) {
        let guard = self.guard.take();
        let admission = self.admission.take();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                if let Some(guard) = guard {
                    let _ = guard.release().await;
                }
                drop(admission);
            });
        } else {
            drop(guard);
            drop(admission);
        }
    }
}

fn unavailable() -> ApiError {
    ApiError::unavailable(
        "private credential renewal is unavailable; its original operation is retained",
    )
}
fn fingerprint(server: &McpServer) -> Result<String, ApiError> {
    Ok(hex::encode(Sha256::digest(
        serde_json::to_vec(&(
            server.id,
            server.tenant_id,
            &server.name,
            &server.url,
            &server.tool_configuration,
        ))
        .map_err(|_| unavailable())?,
    )))
}
fn aad(server: &McpServer) -> String {
    format!(
        "loom:private-mcp:v1:{}:{}:{}",
        server.tenant_id, server.id, server.name
    )
}
fn token(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn scopes(value: &str) -> Option<Vec<&str>> {
    let mut parts: Vec<_> = value.split(' ').collect();
    if parts.is_empty()
        || parts.len() > 32
        || parts
            .iter()
            .any(|p| p.is_empty() || p.len() > 128 || !p.bytes().all(|b| b.is_ascii_graphic()))
    {
        return None;
    }
    parts.sort_unstable();
    if parts.windows(2).any(|p| p[0] == p[1]) {
        return None;
    }
    Some(parts)
}
fn safe_endpoint(server: &McpServer) -> Result<Url, ApiError> {
    let url = Url::parse(&server.url).map_err(|_| unavailable())?;
    let loopback = matches!(
        url.host_str(),
        Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
    );
    if (url.scheme() != "https" && !(url.scheme() == "http" && loopback))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/mcp"
    {
        return Err(unavailable());
    }
    Ok(url)
}
fn http() -> Result<Client, ApiError> {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    Client::builder()
        .redirect(reqwest_mcp::redirect::Policy::none())
        .retry(reqwest_mcp::retry::never())
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|_| unavailable())
}
async fn bounded_json<T: DeserializeOwned>(response: reqwest_mcp::Response) -> Result<T, ApiError> {
    if !response.status().is_success()
        || !response
            .headers()
            .get("content-type")
            .and_then(|s| s.to_str().ok())
            .is_some_and(|s| s.starts_with("application/json"))
    {
        return Err(unavailable());
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| unavailable())?;
        if bytes.len() + chunk.len() > 65536 {
            return Err(unavailable());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| unavailable())
}
async fn lease(
    state: &AppState,
    server: &McpServer,
    exclusive: bool,
) -> Result<McpCredentialLease, ApiError> {
    tokio::time::timeout(
        Duration::from_secs(2),
        state
            .store()
            .lease_mcp_credential(server.tenant_id, &server.name, exclusive),
    )
    .await
    .map_err(|_| unavailable())?
    .map_err(|_| unavailable())?
    .ok_or_else(unavailable)
}
fn decode(
    state: &AppState,
    server: &McpServer,
    nonce: &[u8],
    ciphertext: &[u8],
) -> Result<State, ApiError> {
    let bytes = state
        .crypto()
        .decrypt(nonce, ciphertext, aad(server).as_bytes())
        .map_err(|_| unavailable())?;
    let value: State = serde_json::from_slice(&bytes).map_err(|_| unavailable())?;
    if value.version != 1 || value.registration != fingerprint(server)? || value.fenced {
        return Err(unavailable());
    }
    Ok(value)
}
async fn save(
    state: &AppState,
    server: &McpServer,
    lease: &mut McpCredentialLease,
    revision: Option<i64>,
    value: &State,
) -> Result<i64, ApiError> {
    let bytes = serde_json::to_vec(value).map_err(|_| unavailable())?;
    let sealed = state
        .crypto()
        .encrypt(&bytes, aad(server).as_bytes())
        .map_err(|_| unavailable())?;
    lease
        .save(revision, &sealed.ciphertext, &sealed.nonce)
        .await
        .map_err(|_| unavailable())?
        .ok_or_else(unavailable)
}

/// Install once under root admin authority, after authenticating the exact
/// live account, binding and negotiated private grant. Never exports secrets.
pub(crate) async fn install(
    state: &AppState,
    server: &McpServer,
    credential: Install,
) -> Result<(), ApiError> {
    let _admission = state.admit_conversation()?;
    let mut guard = lease(state, server, true).await?;
    let current = state
        .store()
        .get_mcp_server(server.tenant_id, &server.name)
        .await
        .map_err(|_| unavailable())?
        .ok_or_else(unavailable)?;
    if fingerprint(server)? != fingerprint(&current)? {
        return Err(unavailable());
    }
    let now = Utc::now();
    let cfg = server.tool_configuration.as_ref().ok_or_else(unavailable)?;
    if cfg["enabled"] != true
        || cfg["gateway"]["kind"] != "lucidbrain_memory"
        || serde_json::from_value::<Binding>(cfg["gateway"]["binding"].clone())
            .map_err(|_| unavailable())?
            != credential.binding
        || credential
            .client_id
            .strip_prefix("lb_agent_tandem_")
            .is_none_or(|s| {
                s.len() != 32
                    || !s
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
        || credential.external_account.schema_version != 1
        || credential.external_account.issuer.is_empty()
        || credential.external_account.subject.is_empty()
        || credential.external_account.user_id == credential.binding.principal_id
        || !token(&credential.access_token)
        || !token(&credential.refresh_token)
        || scopes(&credential.scope).is_none()
        || credential.access_expires_at <= now + chrono::Duration::seconds(2 * HORIZON)
        || credential.refresh_expires_at <= credential.access_expires_at
    {
        return Err(unavailable());
    }
    let context: serde_json::Value = bounded_json(
        http()?
            .get(
                safe_endpoint(server)?
                    .join("/mcp/context")
                    .map_err(|_| unavailable())?,
            )
            .bearer_auth(&credential.access_token)
            .header(
                "x-lucidbrain-expected-principal",
                credential.binding.principal_id.to_string(),
            )
            .header(
                "x-lucidbrain-expected-workspace",
                credential.binding.workspace_id.to_string(),
            )
            .header(
                "x-lucidbrain-expected-agent",
                credential.binding.agent_definition_id.to_string(),
            )
            .header(
                "x-lucidbrain-expected-credential",
                credential.binding.credential_id.to_string(),
            )
            .send()
            .await
            .map_err(|_| unavailable())?,
    )
    .await?;
    if credential.access_expires_at<=Utc::now()+chrono::Duration::seconds(2 * HORIZON) || context["schema_version"]!=1 || context["kind"]!="workspace_assistant" || context["recoverable_refresh_version"]!=1
        || serde_json::from_value::<Binding>(serde_json::json!({"principal_id":context["principal_id"],"workspace_id":context["workspace_id"],"agent_definition_id":context["agent_definition_id"],"credential_id":context["credential_id"]})).map_err(|_| unavailable())?!=credential.binding
        || serde_json::from_value::<ExternalAccount>(context["external_account"].clone()).map_err(|_| unavailable())?!=credential.external_account
        || sorted_tools(&context["allowed_tools"])? != sorted_tools(&cfg["allowed_tools"])? {
        return Err(unavailable());
    }
    if let Some((_, cipher, nonce)) = guard.load().await.map_err(|_| unavailable())? {
        // Unknown installation response: accept the same unrotated input only.
        let old = decode(state, server, &nonce, &cipher)?;
        if old.pending.is_some()
            || serde_json::to_value(&old.credential).map_err(|_| unavailable())?
                != serde_json::to_value(&credential).map_err(|_| unavailable())?
        {
            return Err(ApiError::conflict(
                "private credential already installed; controlled recovery required",
            ));
        }
        guard.release().await.map_err(|_| unavailable())?;
        return Ok(());
    }
    save(
        state,
        server,
        &mut guard,
        None,
        &State {
            version: 1,
            registration: fingerprint(server)?,
            credential,
            pending: None,
            fenced: false,
        },
    )
    .await?;
    guard.release().await.map_err(|_| unavailable())?;
    Ok(())
}
fn sorted_tools(value: &serde_json::Value) -> Result<Vec<String>, ApiError> {
    let mut names: Vec<String> =
        serde_json::from_value(value.clone()).map_err(|_| unavailable())?;
    names.sort();
    if names.is_empty() || names.len() > 5 || names.windows(2).any(|v| v[0] == v[1]) {
        return Err(unavailable());
    }
    Ok(names)
}

/// Resolve at the start of a logical run. Legacy registrations retain their
/// existing path; a private row never falls back to a static bearer.
pub(crate) async fn authorization(
    state: &AppState,
    server: &McpServer,
) -> Result<(String, Option<AuthorizationLease>), ApiError> {
    if !state
        .store()
        .has_private_mcp_credential(server.tenant_id, &server.name)
        .await
        .map_err(|_| unavailable())?
    {
        return Ok((
            crate::provider::decrypt_mcp_token(state, server.tenant_id, &server.name, server)?
                .ok_or_else(unavailable)?,
            None,
        ));
    }
    let admission = state.admit_conversation()?;
    let mut guard = lease(state, server, false).await?;
    let (_, cipher, nonce) = guard
        .load()
        .await
        .map_err(|_| unavailable())?
        .ok_or_else(unavailable)?;
    let value = decode(state, server, &nonce, &cipher)?;
    if value.pending.is_none()
        && value.credential.access_expires_at > Utc::now() + chrono::Duration::seconds(HORIZON)
    {
        return Ok((
            value.credential.access_token,
            Some(AuthorizationLease {
                guard: Some(guard),
                admission: Some(admission),
            }),
        ));
    }
    guard.release().await.map_err(|_| unavailable())?;
    // A competing active client delays renewal; never rotate beneath it.
    let mut guard = lease(state, server, true).await?;
    let (mut revision, cipher, nonce) = guard
        .load()
        .await
        .map_err(|_| unavailable())?
        .ok_or_else(unavailable)?;
    let mut value = decode(state, server, &nonce, &cipher)?;
    // At most recover one expired generation and then rotate it once.
    for _ in 0..2 {
        if value.pending.is_none()
            && value.credential.access_expires_at > Utc::now() + chrono::Duration::seconds(HORIZON)
        {
            return Ok((
                value.credential.access_token,
                Some(AuthorizationLease {
                    guard: Some(guard),
                    admission: Some(admission),
                }),
            ));
        }
        if value.credential.refresh_expires_at <= Utc::now() {
            return Err(unavailable());
        }
        let operation = match value.pending {
            Some(id) => id,
            None => {
                let id = Uuid::new_v4();
                value.pending = Some(id);
                revision = save(state, server, &mut guard, Some(revision), &value).await?;
                id
            }
        };
        let response = http()?.post(safe_endpoint(server)?.join("/api/v1/oauth/token").map_err(|_| unavailable())?)
            .json(&serde_json::json!({"grant_type":GRANT,"client_id":value.credential.client_id,"credential_id":value.credential.binding.credential_id,"operation_id":operation,"refresh_token":value.credential.refresh_token}))
            .send().await.map_err(|_| unavailable())?;
        let renewal: Renewal = bounded_json(response).await?;
        validate_renewal(&value.credential, operation, &renewal)?;
        value.credential.access_token = renewal.access_token;
        value.credential.refresh_token = renewal.refresh_token;
        value.credential.access_expires_at = renewal.access_expires_at;
        value.credential.refresh_expires_at = renewal.refresh_expires_at;
        value.pending = None;
        value.fenced = renewal.state == "active"
            && renewal.access_expires_at <= Utc::now() + chrono::Duration::seconds(HORIZON);
        revision = save(state, server, &mut guard, Some(revision), &value).await?;
        if value.fenced {
            return Err(unavailable());
        }
    }
    if value.credential.access_expires_at > Utc::now() + chrono::Duration::seconds(HORIZON) {
        Ok((
            value.credential.access_token,
            Some(AuthorizationLease {
                guard: Some(guard),
                admission: Some(admission),
            }),
        ))
    } else {
        Err(unavailable())
    }
}
fn validate_renewal(old: &Install, operation: Uuid, new: &Renewal) -> Result<(), ApiError> {
    let now = Utc::now();
    let remaining = (new.access_expires_at - now).num_seconds();
    if new.grant_version != 1
        || new.operation_id != operation
        || new.client_id != old.client_id
        || new.credential_id != old.binding.credential_id
        || new.token_type != "Bearer"
        || new.binding != old.binding
        || new.external_account != old.external_account
        || !token(&new.access_token)
        || !token(&new.refresh_token)
        || new.access_token == old.access_token
        || new.refresh_token == old.refresh_token
        || scopes(&new.scope).is_none()
        || scopes(&new.scope) != scopes(&old.scope)
        || new.refresh_expires_at <= now
        || new.refresh_expires_at <= new.access_expires_at
        || new.recovery_expires_at <= now
        || new.recovery_expires_at > old.refresh_expires_at
        || new.recovery_expires_at > now + chrono::Duration::seconds(3600)
        || new.expires_in < 0
        || new.expires_in > 86400
        || new.access_expires_at > now + chrono::Duration::seconds(86400)
        || (new.expires_in - remaining.max(0)).abs() > 5
        || !((new.state == "active" && remaining > 0 && new.expires_in > 0)
            || (new.state == "refresh_required" && remaining <= 0 && new.expires_in == 0))
    {
        return Err(unavailable());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixtures() -> (Install, Uuid, serde_json::Value) {
        let now = Utc::now();
        let binding = Binding {
            principal_id: Uuid::new_v4(),
            workspace_id: Uuid::new_v4(),
            agent_definition_id: Uuid::new_v4(),
            credential_id: Uuid::new_v4(),
        };
        let external = ExternalAccount {
            schema_version: 1,
            account_id: Uuid::new_v4(),
            issuer: "https://cognito.invalid/pool".into(),
            subject: "opaque".into(),
            user_id: Uuid::new_v4(),
            default_category_id: Uuid::new_v4(),
        };
        let old = Install {
            client_id: format!("lb_agent_tandem_{}", Uuid::new_v4().simple()),
            binding,
            external_account: external,
            access_token: "a".repeat(64),
            refresh_token: "b".repeat(64),
            scope: "read write".into(),
            access_expires_at: now + chrono::Duration::minutes(1),
            refresh_expires_at: now + chrono::Duration::days(30),
        };
        let operation = Uuid::new_v4();
        let new = json!({"grant_version":1,"operation_id":operation,"client_id":old.client_id,"credential_id":old.binding.credential_id,"token_type":"Bearer","access_token":"c".repeat(64),"refresh_token":"d".repeat(64),"expires_in":3600,"access_expires_at":now+chrono::Duration::hours(1),"refresh_expires_at":now+chrono::Duration::days(30),"recovery_expires_at":now+chrono::Duration::minutes(10),"scope":"write read","state":"active","binding":old.binding,"external_account":old.external_account});
        (old, operation, new)
    }
    #[test]
    fn renewal_validates_identity_scope_pair_operation_and_deadlines() {
        let (old, operation, new) = fixtures();
        assert!(validate_renewal(
            &old,
            operation,
            &serde_json::from_value(new.clone()).unwrap()
        )
        .is_ok());
        for (key, value) in [
            ("grant_version", json!(2)),
            ("operation_id", json!(Uuid::new_v4())),
            ("client_id", json!("other")),
            ("credential_id", json!(Uuid::new_v4())),
            ("token_type", json!("Other")),
            ("access_token", json!(old.access_token)),
            ("refresh_token", json!(old.refresh_token)),
            ("scope", json!("read write admin")),
            ("scope", json!("read read")),
            ("expires_in", json!(-1)),
            ("expires_in", json!(86401)),
            ("state", json!("unknown")),
            (
                "recovery_expires_at",
                json!(Utc::now() - chrono::Duration::seconds(1)),
            ),
            (
                "recovery_expires_at",
                json!(Utc::now() + chrono::Duration::hours(2)),
            ),
            (
                "refresh_expires_at",
                json!(Utc::now() - chrono::Duration::seconds(1)),
            ),
        ] {
            let mut altered = new.clone();
            altered[key] = value;
            assert!(
                validate_renewal(&old, operation, &serde_json::from_value(altered).unwrap())
                    .is_err(),
                "{key} must fail closed"
            );
        }
        for key in [
            "principal_id",
            "workspace_id",
            "agent_definition_id",
            "credential_id",
        ] {
            let mut altered = new.clone();
            altered["binding"][key] = json!(Uuid::new_v4());
            assert!(
                validate_renewal(&old, operation, &serde_json::from_value(altered).unwrap())
                    .is_err()
            );
        }
        for key in ["account_id", "user_id", "default_category_id"] {
            let mut altered = new.clone();
            altered["external_account"][key] = json!(Uuid::new_v4());
            assert!(
                validate_renewal(&old, operation, &serde_json::from_value(altered).unwrap())
                    .is_err()
            );
        }
        for key in ["issuer", "subject"] {
            let mut altered = new.clone();
            altered["external_account"][key] = json!("other");
            assert!(
                validate_renewal(&old, operation, &serde_json::from_value(altered).unwrap())
                    .is_err()
            );
        }
    }
    #[test]
    fn expired_recovered_generation_is_valid_only_as_refresh_required() {
        let (old, operation, mut new) = fixtures();
        new["access_expires_at"] = json!(Utc::now() - chrono::Duration::seconds(10));
        new["expires_in"] = json!(0);
        assert!(validate_renewal(
            &old,
            operation,
            &serde_json::from_value(new.clone()).unwrap()
        )
        .is_err());
        new["state"] = json!("refresh_required");
        assert!(validate_renewal(&old, operation, &serde_json::from_value(new).unwrap()).is_ok());
    }
}
