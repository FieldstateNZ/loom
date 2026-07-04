//! A reference to an external MCP server.

use serde::{Deserialize, Serialize};

/// A reference to an external MCP server the model may use via a provider
/// connector.
///
/// Two forms are supported:
///
/// - **Named reference** — set only [`name`](Self::name) (and optionally
///   [`tool_configuration`](Self::tool_configuration)). The gateway resolves the
///   name against the calling tenant's registered MCP servers, loads the stored
///   URL, and injects the decrypted authorization token **server-side** at
///   request time. This is the recommended path: an MCP auth token never leaves
///   the gateway, never transits the client, and never appears in a response or
///   in persisted history.
/// - **Inline** — additionally set [`url`](Self::url) and (optionally)
///   [`authorization`](Self::authorization) to drive a server directly without
///   registering it. The advanced path, for callers that manage their own MCP
///   credentials.
///
/// # Secret handling
///
/// [`authorization`](Self::authorization) is a bearer token. It is deliberately
/// **redacted from the [`Debug`] representation** so it cannot leak into logs,
/// and it is never serialized back to a client (request options are inbound
/// only; they are not part of the persisted [`Conversation`](crate::Conversation)
/// or any response body).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerRef {
    /// The server's logical name. For a named reference this selects the
    /// tenant's registered server; it is also sent to the provider as the
    /// server's identifier so response tool blocks can be correlated.
    pub name: String,

    /// The MCP server endpoint URL. Absent for a named reference (the gateway
    /// fills it in from the registry); present for an inline server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

    /// A bearer authorization token for the server. For a named reference this
    /// is left unset by the caller and injected server-side after decryption;
    /// for an inline server the caller supplies it directly. Redacted from
    /// [`Debug`]; never persisted or returned.
    ///
    /// The field is **deserialize-only**: it is read from an inbound request but
    /// [`skipped on serialization`](serde), so a decrypted token can never leak
    /// out of a serialized [`ConversationOptions`](super::ConversationOptions)
    /// (e.g. request telemetry that renders the options to JSON). The
    /// guarantee is structural, not by convention.
    #[serde(default, skip_serializing)]
    pub authorization: Option<String>,

    /// Optional provider-native tool-configuration for this server (e.g. an
    /// allow-list of tool names), forwarded verbatim. Absent when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_configuration: Option<serde_json::Value>,
}

impl McpServerRef {
    /// Constructs a **named reference** to a registered MCP server. The
    /// gateway resolves the URL and injects the authorization token
    /// server-side.
    #[must_use]
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: None,
            authorization: None,
            tool_configuration: None,
        }
    }

    /// Constructs an **inline** reference driving a server directly by URL,
    /// with an optional authorization token the caller manages itself.
    #[must_use]
    pub fn inline(
        name: impl Into<String>,
        url: impl Into<String>,
        authorization: Option<String>,
    ) -> Self {
        Self {
            name: name.into(),
            url: Some(url.into()),
            authorization,
            tool_configuration: None,
        }
    }
}

impl std::fmt::Debug for McpServerRef {
    /// Redacts [`authorization`](Self::authorization) so an MCP bearer token
    /// never appears in a log line, panic message, or test failure.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpServerRef")
            .field("name", &self.name)
            .field("url", &self.url)
            .field(
                "authorization",
                &self.authorization.as_ref().map(|_| "<redacted>"),
            )
            .field("tool_configuration", &self.tool_configuration)
            .finish()
    }
}
