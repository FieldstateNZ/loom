//! Private, identity-bound LucidBrain access. Inference can read reviewed memory
//! and optionally propose a task. Actual task delivery is a separate host API.
//! No provider credential forwarding, redirects, transparent retries or sampling.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::{collections::BTreeSet, sync::Arc, time::Duration};

use futures::StreamExt;
use loom_core::ToolDefinition;
use reqwest_mcp::{
    header::{HeaderMap, HeaderValue},
    Client, Url,
};
use rmcp::{
    model::{
        CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation, ProtocolVersion,
    },
    service::{RoleClient, RunningService},
    transport::{
        common::client_side_sse::NeverRetry,
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    ServiceExt,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

mod arguments;
mod tasks;
pub use tasks::{CaptureReceipt, TaskCategory, TaskInput};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Memory,
    Tasks,
    Receipts,
}
impl Mode {
    fn permits(self, name: &str) -> bool {
        is_memory_read(name)
            || (self != Self::Memory
                && matches!(
                    name,
                    "list_categories" | "capture_item" | "get_capture_operation"
                ))
    }
}

/// Server-owned identity expected for every MCP request; never model input.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct Binding {
    /// LucidBrain acting principal (normally an agent service user).
    pub principal_id: Uuid,
    /// Workspace hard-pinned by the credential.
    pub workspace_id: Uuid,
    /// LucidBrain assistant definition.
    pub agent_definition_id: Uuid,
    /// Stable credential identity, independent of access-token refresh.
    pub credential_id: Uuid,
}

/// A deliberately non-diagnostic error: no URL, token, server body or SDK
/// error is included in Debug, Display or the gateway's error response.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Unsafe or incomplete operator configuration.
    #[error("private memory connection is not configured")]
    Configuration,
    /// Authenticated identity or policy differs from the provisioned binding.
    #[error("private memory identity or permission mismatch")]
    Binding,
    /// The peer failed, timed out or sent an unsupported response.
    #[error("private memory service could not complete the request")]
    Transport,
    /// An unoffered tool or invalid arguments were requested.
    #[error("memory tool request is not permitted")]
    Tool,
}

#[derive(Deserialize)]
struct Context {
    schema_version: u32,
    kind: String,
    principal_id: Uuid,
    workspace_id: Uuid,
    agent_definition_id: Uuid,
    credential_id: Uuid,
    allowed_tools: Option<Vec<String>>,
    transport_mode: String,
    capture_operation_version: Option<u32>,
}

/// Only the two source-reviewed LucidBrain read tools are currently executable.
#[must_use]
pub fn is_memory_read(name: &str) -> bool {
    matches!(name, "memory_recall" | "memory_search")
}

/// An authenticated private MCP connection, freshly created for a logical turn
/// or host action command. The historical name preserves the read-client API.
/// The official SDK owns the legacy handshake and JSON/SSE protocol handling.
/// Dropping the connection cancels its service; no tool request is retried.
pub struct MemoryClient {
    mode: Mode,
    http: Client,
    context_url: Url,
    headers: HeaderMap,
    binding: Binding,
    service: RunningService<RoleClient, ClientConfig>,
    tools: Vec<ToolDefinition>,
}

impl MemoryClient {
    /// Verifies the actual credential identity, requires a finite read-only
    /// policy and discovers tool schemas before any model inference.
    pub async fn connect(
        endpoint: &str,
        token: &str,
        binding: Binding,
        registered_tools: &[String],
        requested_tools: &[String],
    ) -> Result<Self, Error> {
        Self::connect_mode(
            endpoint,
            token,
            binding,
            registered_tools,
            requested_tools,
            Mode::Memory,
        )
        .await
    }

    /// Connects the reviewed task contract. Only memory reads and a synthetic
    /// proposal tool are exposed to inference; capture is a separate host API.
    pub async fn connect_tasks(
        endpoint: &str,
        token: &str,
        binding: Binding,
        registered_tools: &[String],
        memory_tools: &[String],
    ) -> Result<Self, Error> {
        if memory_tools.iter().any(|n| !is_memory_read(n)) {
            return Err(Error::Configuration);
        }
        let mut required = memory_tools.to_vec();
        required
            .extend(["list_categories", "capture_item", "get_capture_operation"].map(String::from));
        let mut client = Self::connect_mode(
            endpoint,
            token,
            binding,
            registered_tools,
            &required,
            Mode::Tasks,
        )
        .await?;
        client.tools.retain(|t| is_memory_read(&t.name));
        client.tools.push(tasks::proposal_tool());
        Ok(client)
    }

    /// Read-only receipt connection, usable after write permission is removed.
    pub async fn connect_receipts(
        endpoint: &str,
        token: &str,
        binding: Binding,
        registered_tools: &[String],
    ) -> Result<Self, Error> {
        Self::connect_mode(
            endpoint,
            token,
            binding,
            registered_tools,
            &["get_capture_operation".into()],
            Mode::Receipts,
        )
        .await
    }

    async fn connect_mode(
        endpoint: &str,
        token: &str,
        binding: Binding,
        registered_tools: &[String],
        requested_tools: &[String],
        mode: Mode,
    ) -> Result<Self, Error> {
        let endpoint = checked_endpoint(endpoint)?;
        if token.is_empty()
            || requested_tools.is_empty()
            || registered_tools.is_empty()
            || registered_tools.iter().any(|n| !mode.permits(n))
            || requested_tools
                .iter()
                .any(|n| !registered_tools.contains(n))
        {
            return Err(Error::Configuration);
        }
        let mut headers = HeaderMap::new();
        let mut auth =
            HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| Error::Configuration)?;
        auth.set_sensitive(true);
        headers.insert("authorization", auth);
        for (name, id) in [
            ("x-lucidbrain-expected-principal", binding.principal_id),
            ("x-lucidbrain-expected-workspace", binding.workspace_id),
            ("x-lucidbrain-expected-agent", binding.agent_definition_id),
            ("x-lucidbrain-expected-credential", binding.credential_id),
        ] {
            headers.insert(
                name,
                HeaderValue::from_str(&id.to_string()).map_err(|_| Error::Configuration)?,
            );
        }
        // reqwest's no-provider feature requires explicit initialization. Keep
        // any process-selected provider; otherwise use the same ring backend
        // as Loom's existing HTTP clients. A concurrent initializer is safe.
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            let _ = rustls::crypto::ring::default_provider().install_default();
        }
        let http = Client::builder()
            .redirect(reqwest_mcp::redirect::Policy::none())
            .retry(reqwest_mcp::retry::never())
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|_| Error::Configuration)?;
        let mut context_url = endpoint.clone();
        context_url.set_path(&format!("{}/context", endpoint.path()));
        let policy = attest(&http, &context_url, &headers, &binding, mode).await?;
        if requested_tools.iter().any(|n| !policy.contains(n)) {
            return Err(Error::Binding);
        }
        let mut config = StreamableHttpClientTransportConfig::with_uri(endpoint.as_str())
            .max_concurrent_requests(1)
            .reinit_on_expired_session(false)
            .max_sse_event_size(1024 * 1024);
        config.allow_stateless = true;
        config.retry_config = Arc::new(NeverRetry::default());
        config.custom_headers = headers
            .clone()
            .into_iter()
            .filter_map(|(name, value)| name.map(|name| (name, value)))
            .collect();
        let transport = StreamableHttpClientTransport::with_client(http.clone(), config);
        let info = ClientConfig::new(
            ClientCapabilities::default(),
            Implementation::new("loom-private-memory", "0.1.0"),
        )
        .with_protocol_version(ProtocolVersion::V_2025_03_26);
        let service = tokio::time::timeout(Duration::from_secs(6), info.serve(transport))
            .await
            .map_err(|_| Error::Transport)?
            .map_err(|_| Error::Transport)?;
        let listed = tokio::time::timeout(Duration::from_secs(5), service.list_tools(None))
            .await
            .map_err(|_| Error::Transport)?
            .map_err(|_| Error::Transport)?;
        if listed.next_cursor.is_some()
            || listed.tools.len() > if mode == Mode::Memory { 2 } else { 5 }
        {
            return Err(Error::Binding);
        }
        let names: BTreeSet<_> = listed.tools.iter().map(|t| t.name.to_string()).collect();
        if names.len() != listed.tools.len() || names != policy {
            return Err(Error::Binding);
        }
        let mut tools = Vec::new();
        for tool in listed.tools {
            if !requested_tools.iter().any(|n| n == tool.name.as_ref()) {
                continue;
            }
            let schema = serde_json::to_value(tool.input_schema).map_err(|_| Error::Transport)?;
            if schema.get("type").and_then(Value::as_str) != Some("object")
                || schema.to_string().len() > 16384
                || tool.description.as_ref().is_some_and(|s| s.len() > 4096)
            {
                return Err(Error::Binding);
            }
            tools.push(ToolDefinition {
                name: tool.name.to_string(),
                description: tool.description.map(|s| s.to_string()),
                input_schema: schema,
                cache: None,
            });
        }
        Ok(Self {
            mode,
            http,
            context_url,
            headers,
            binding,
            service,
            tools,
        })
    }

    /// Definitions that may be exposed to the model. No URLs or credentials.
    #[must_use]
    pub fn tools(&self) -> &[ToolDefinition] {
        &self.tools
    }

    /// Checks a proposed call without network I/O, so an entire model-issued
    /// batch can be refused before any of its calls is executed.
    pub fn validate(&self, name: &str, input: &Value) -> Result<(), Error> {
        if !self.tools.iter().any(|t| t.name == name) {
            return Err(Error::Tool);
        }
        if name == "propose_task" && self.mode == Mode::Tasks {
            TaskInput::parse(input).map(|_| ())
        } else {
            arguments::validate(name, input)
        }
    }

    /// Validates authorization and arguments before dispatching a read once.
    /// Tool diagnostics are replaced with a safe error result, not sent to the
    /// model. Sampling, input-required rounds and asynchronous tasks are refused.
    pub async fn call(&self, name: &str, input: &Value) -> Result<(Value, bool), Error> {
        self.validate(name, input)?;
        if !is_memory_read(name) {
            return Err(Error::Tool);
        }
        self.call_private(name, input).await
    }

    async fn call_private(&self, name: &str, input: &Value) -> Result<(Value, bool), Error> {
        let current = attest(
            &self.http,
            &self.context_url,
            &self.headers,
            &self.binding,
            self.mode,
        )
        .await?;
        if !current.contains(name) {
            return Err(Error::Binding);
        }
        let params = CallToolRequestParams::new(name.to_owned())
            .with_arguments(input.as_object().cloned().ok_or(Error::Tool)?);
        let response =
            tokio::time::timeout(Duration::from_secs(5), self.service.call_tool_once(params))
                .await
                .map_err(|_| Error::Transport)?
                .map_err(|_| Error::Transport)?;
        let rmcp::model::CallToolResponse::Complete(result) = response else {
            return Err(Error::Transport);
        };
        if result.is_error == Some(true) {
            return Ok((serde_json::json!({"error":"Memory read failed"}), true));
        }
        let value = serde_json::to_value(result).map_err(|_| Error::Transport)?;
        if value.to_string().len() > 1024 * 1024
            || value["content"]
                .as_array()
                .is_none_or(|parts| parts.iter().any(|p| p["type"] != "text"))
        {
            return Err(Error::Transport);
        }
        Ok((value, false))
    }
}

async fn attest(
    http: &Client,
    url: &Url,
    headers: &HeaderMap,
    binding: &Binding,
    mode: Mode,
) -> Result<BTreeSet<String>, Error> {
    let response = http
        .get(url.clone())
        .headers(headers.clone())
        .send()
        .await
        .map_err(|_| Error::Transport)?;
    if !response.status().is_success() {
        return Err(Error::Binding);
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| Error::Transport)?;
        if bytes.len() + chunk.len() > 16384 {
            return Err(Error::Transport);
        }
        bytes.extend_from_slice(&chunk);
    }
    let ctx: Context = serde_json::from_slice(&bytes).map_err(|_| Error::Transport)?;
    if ctx.schema_version != 1
        || ctx.kind != "workspace_assistant"
        || ctx.transport_mode != "stateless"
        || ctx.principal_id != binding.principal_id
        || ctx.workspace_id != binding.workspace_id
        || ctx.agent_definition_id != binding.agent_definition_id
        || ctx.credential_id != binding.credential_id
    {
        return Err(Error::Binding);
    }
    if mode != Mode::Memory && ctx.capture_operation_version != Some(1) {
        return Err(Error::Binding);
    }
    let tools = ctx.allowed_tools.ok_or(Error::Binding)?;
    if tools.is_empty()
        || tools.len() > if mode == Mode::Memory { 2 } else { 5 }
        || tools.iter().any(|n| !mode.permits(n))
    {
        return Err(Error::Binding);
    }
    let set: BTreeSet<_> = tools.into_iter().collect();
    Ok(set)
}

fn checked_endpoint(value: &str) -> Result<Url, Error> {
    let url = Url::parse(value).map_err(|_| Error::Configuration)?;
    let local = matches!(
        url.host_str(),
        Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
    );
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !(url.scheme() == "https" || (url.scheme() == "http" && local))
        || !url.path().ends_with("/mcp")
    {
        return Err(Error::Configuration);
    }
    Ok(url)
}
