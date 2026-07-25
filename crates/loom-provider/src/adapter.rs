//! The OASP `AgentProvider` adapter contract.
//!
//! Where [`Provider`](crate::Provider) is Loom's per-turn translation trait, an
//! [`AgentProvider`] is the OASP adapter contract: a session-lifecycle
//! interface a server drives without knowing which provider is underneath. Its
//! operations map a provider into the managed-agent model — version pinning,
//! resource/vault fidelity, the normalised event stream, pending-tool
//! enumeration.
//!
//! This slice defines the **contract** only; a concrete Anthropic
//! implementation is a later slice. Each operation's full normative behaviour —
//! including the preserve-vs-may-lose boundary — lives in the OASP adapter spec
//! (`docs/spec/adapters.md` in `oasp-standard`, vendored read-only at
//! `docs/oasp-spec/spec/adapters.md` in this repo); the doc comments here are a
//! summary, not a substitute.
//!
//! ## Session identity convention
//!
//! Every operation below keys a session on `session_id: &str` — a
//! **provider-assigned string id**, not Loom's internal persistence-layer
//! [`Uuid`]. [`AgentProvider::create_session`] returns an [`AdapterSession`]
//! (not [`loom_core::Session`]) whose [`id`](AdapterSession::id) is exactly
//! that string: a caller passes it straight into `send_message`,
//! `send_tool_result`, `get_session_status`, `list_session_events`,
//! `stream_events` and `get_pending_tool_calls` as `session_id`, with no
//! UUID/string impedance mismatch. Mapping that provider-assigned id onto
//! Loom's own `Session` persistence record (whose `id` is a server-generated
//! `Uuid` primary key) is a later slice's job — this contract only fixes the
//! adapter boundary's own shape.
//!
//! ## `utoipa::ToSchema` gating
//!
//! None of this module's DTOs derive `utoipa::ToSchema`, even under the
//! `openapi` feature. This is deliberate, not an oversight: several of them
//! ([`SeedTranscript`], [`ListSessionEventsResult`]) embed
//! [`loom_core::Event`], which itself has no `ToSchema` impl (a pre-existing
//! gap in `loom-core`'s own `event.rs`, predating this contract and out of
//! this slice's scope to fix). Gating only the DTOs that happen not to embed
//! `Event` would be the "half-and-half" state to avoid, so every DTO here is
//! left ungated uniformly until `Event` itself gets a schema.

use async_trait::async_trait;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use loom_core::{AgentDefinition, AgentVersionRef, Deployment, Event, RunStatus, SessionResource};

/// The error every fallible [`AgentProvider`] operation returns on failure.
///
/// `retryable` distinguishes a transient provider hiccup (try again) from a
/// permanent failure (unknown session, malformed input) without parsing `code`.
/// It MUST be set deliberately per error, never defaulted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AdapterError {
    /// Stable, machine-readable error code (e.g. `Adapter.SessionNotFound`).
    pub code: String,
    /// Human-readable description of the failure.
    pub message: String,
    /// Whether the operation may succeed if retried as posed.
    pub retryable: bool,
}

impl AdapterError {
    /// No session exists with the given id.
    #[must_use]
    pub fn session_not_found(session_id: &str) -> Self {
        Self {
            code: "Adapter.SessionNotFound".to_owned(),
            message: format!("No session found with id \"{session_id}\"."),
            retryable: false,
        }
    }

    /// No provider-side agent exists with the given id.
    #[must_use]
    pub fn agent_not_found(provider_agent_id: &str) -> Self {
        Self {
            code: "Adapter.AgentNotFound".to_owned(),
            message: format!("No provider agent found with id \"{provider_agent_id}\"."),
            retryable: false,
        }
    }

    /// `send_tool_result` was called with a `tool_use_id` that has no matching
    /// pending tool use.
    #[must_use]
    pub fn unknown_tool_use(tool_use_id: &str) -> Self {
        Self {
            code: "Adapter.UnknownToolUse".to_owned(),
            message: format!("No pending tool use found with id \"{tool_use_id}\"."),
            retryable: false,
        }
    }

    /// A transcript fetch (which `list_session_events` powers) failed —
    /// exercises `migrate`'s degrade-to-fresh-start path.
    #[must_use]
    pub fn transcript_fetch_failed(session_id: &str) -> Self {
        Self {
            code: "Adapter.TranscriptFetchFailed".to_owned(),
            message: format!("Failed to fetch the transcript for session \"{session_id}\"."),
            retryable: true,
        }
    }

    /// A generic, test-induced failure — used by a mock provider's controls to
    /// simulate an arbitrary operation failing on demand.
    #[must_use]
    pub fn induced_failure(operation: &str) -> Self {
        Self {
            code: "Adapter.InducedFailure".to_owned(),
            message: format!("Induced failure for operation \"{operation}\" (test-controlled)."),
            retryable: true,
        }
    }
}

impl std::fmt::Display for AdapterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for AdapterError {}

/// A transcript to seed into a new session as already-exchanged content —
/// `migrate`'s Stage 2.
///
/// The events arrive already flattened and non-compounded by the server; an
/// adapter treats them as one flat batch and MUST NOT produce a fresh assistant
/// turn as an unsolicited response to the seed alone.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SeedTranscript {
    /// The ordered events to seed, as already-exchanged history.
    pub events: Vec<Event>,
}

/// Input to [`AgentProvider::create_session`]. Every field the returned
/// [`AdapterSession`] must echo back faithfully.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CreateSessionOptions {
    /// The agent definition this session is being created for. MUST equal
    /// [`pinned_agent_version`](CreateSessionOptions::pinned_agent_version)'s
    /// `agent_definition_id`; typed as a `Uuid` (not a string) so it cannot
    /// admit a malformed value. The two are independent fields, so a server
    /// validates that they agree when it builds these options.
    pub agent_definition_id: Uuid,
    /// The provider-side agent id (from a prior create/update) to create the
    /// session against.
    pub provider_agent_id: String,
    /// The exact agent version to pin the session to — preserved verbatim,
    /// never re-resolved adapter-side.
    pub pinned_agent_version: AgentVersionRef,
    /// Resources to mount at creation, in full (never partially).
    pub resources: Vec<SessionResource>,
    /// Credential vault ids to attach at creation, in full (never partially).
    pub vault_ids: Vec<String>,
    /// A transcript to seed, if this session continues prior history (migrate);
    /// absent for a brand-new session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<SeedTranscript>,
}

/// The provider-facing session representation returned by
/// [`AgentProvider::create_session`].
///
/// Deliberately **not** [`loom_core::Session`]: that type is Loom's internal
/// persistence record, keyed on a server-generated [`Uuid`] primary key with
/// tenant/conversation bookkeeping the adapter boundary has no business
/// exposing. Per `docs/spec/adapters.md` § `createSession`, the returned
/// session "MUST carry the provider-assigned `id`" and echo back
/// `pinnedAgentVersion`/`resources`/`vaultIds` exactly as requested — this type
/// carries exactly that, and nothing else. See the [module-level
/// docs](self) for the full session-identity convention this establishes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdapterSession {
    /// The provider-assigned session id. Pass this directly as `session_id` to
    /// every other [`AgentProvider`] operation — it is a plain string, never a
    /// [`Uuid`] to parse or reinterpret.
    pub id: String,
    /// Echoes [`CreateSessionOptions::pinned_agent_version`] exactly.
    pub pinned_agent_version: AgentVersionRef,
    /// Echoes [`CreateSessionOptions::resources`] exactly.
    pub resources: Vec<SessionResource>,
    /// Echoes [`CreateSessionOptions::vault_ids`] exactly.
    pub vault_ids: Vec<String>,
}

/// Result of [`AgentProvider::ensure_environment`]: confirmation the named
/// environment exists and is ready to host agents.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct EnsureEnvironmentResult {
    /// Echoes the requested environment id.
    pub environment_id: String,
}

/// Pagination input to [`AgentProvider::list_session_events`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ListSessionEventsOptions {
    /// Return only events emitted strictly after this event id; `None` starts
    /// from the beginning of the session's history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_id: Option<String>,
    /// Maximum number of events to return in this page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<i64>,
}

/// A page of a session's normalised event history, in emission order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ListSessionEventsResult {
    /// The events in this page, ordered by emission (equivalently by `id`).
    pub events: Vec<Event>,
    /// The `id` to pass as the next call's `after_id`, or `None` once this page
    /// reached the end of the currently-known history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// A blocking tool use a session is currently parked on, as enumerated by
/// [`AgentProvider::get_pending_tool_calls`].
///
/// Deliberately **not** [`loom_core::PendingToolCall`]: that type is Loom's
/// internal persistence DTO, carrying server bookkeeping (`id`, `session_id`,
/// `created_at`) the adapter boundary has no business exposing. This type
/// carries exactly the fields the canonical `PendingToolCall`
/// (`packages/conformance/src/adapter/pending-tool-call.types.ts` in
/// `oasp-standard`, vendored at
/// `docs/oasp-spec/adapter-types/pending-tool-call.types.ts`) defines — a
/// conformant adapter MUST preserve every one of them faithfully, since
/// `drain` cannot execute a tool call it cannot correctly identify, and cannot
/// authorize one it cannot correctly attribute to an MCP server either.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdapterPendingToolCall {
    /// Correlates this tool use to the `send_tool_result` call that resolves
    /// it.
    pub tool_use_id: String,
    /// Name of the tool being invoked (custom or builtin).
    pub name: String,
    /// The input arguments passed to the tool.
    pub input: serde_json::Value,
    /// The MCP server URL this call was routed through, when the adapter can
    /// identify one — provenance `drain`'s pre-dispatch authorization step
    /// uses to match the call against a granted `mcp` tool's `server_url` and
    /// `tool_allowlist`. Absent for a builtin/custom call, or for an MCP
    /// integration whose adapter genuinely cannot attribute a call to a
    /// specific server — canonical's "MAY lose" clause still permits that
    /// absence. This field MUST remain on the type (it is load-bearing for
    /// `drain`'s MCP authorization) even though its *value* is optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_server_url: Option<String>,
}

/// A stream of normalised session events, boxed so the trait stays object-safe.
///
/// Per the contract, `stream_events` yields [`Event`]s directly (not `Result`s):
/// a failure is surfaced as an `error` Event within the stream, and the stream
/// terminates once a `status: idle` or non-recoverable `error` Event is yielded.
pub type EventStream = BoxStream<'static, Event>;

/// The OASP adapter contract: the interface every provider integration
/// (Anthropic, and later OpenAI/Google) must satisfy so a server can drive it
/// without knowing which provider is underneath.
///
/// Every fallible operation returns `Result<_, AdapterError>`; `stream_events`
/// is the sole exception — an event stream has no place for a `Result` wrapper,
/// so it surfaces failures as `error` events within the stream instead.
///
/// Every operation keys a session on `session_id: &str` — a **provider-assigned
/// string id**, echoed back from [`create_session`](Self::create_session) as
/// [`AdapterSession::id`]. See the [module-level docs](self) for the full
/// session-identity convention.
#[async_trait]
pub trait AgentProvider: Send + Sync {
    /// Idempotently ensures the named provider-side environment exists; MUST be
    /// safe to call repeatedly for the same `environment_id`.
    async fn ensure_environment(
        &self,
        environment_id: &str,
    ) -> Result<EnsureEnvironmentResult, AdapterError>;

    /// Materialises `definition` at the provider within `environment_id`,
    /// returning the resulting deployment.
    async fn create_agent(
        &self,
        definition: &AgentDefinition,
        environment_id: &str,
    ) -> Result<Deployment, AdapterError>;

    /// Updates the provider-side agent identified by `provider_agent_id` to
    /// match `definition`, in place — MUST NOT create a second agent.
    async fn update_agent(
        &self,
        provider_agent_id: &str,
        definition: &AgentDefinition,
        environment_id: &str,
    ) -> Result<Deployment, AdapterError>;

    /// Fetches the current deployment for `provider_agent_id` without mutating
    /// provider state.
    async fn get_agent(&self, provider_agent_id: &str) -> Result<Deployment, AdapterError>;

    /// Creates a provider execution context pinned to
    /// `options.pinned_agent_version`, with resources and vaults mounted in
    /// full and any `seed` applied as already-exchanged content. The returned
    /// [`AdapterSession`] MUST carry the provider-assigned `id` and echo the
    /// pin, resources and vaults exactly as requested.
    async fn create_session(
        &self,
        options: CreateSessionOptions,
    ) -> Result<AdapterSession, AdapterError>;

    /// Posts `content` into the named session as a new turn, attributed to
    /// `principal` where the provider supports per-turn attribution.
    async fn send_message(
        &self,
        session_id: &str,
        content: &str,
        principal: Option<&str>,
    ) -> Result<(), AdapterError>;

    /// Posts `result` for the pending tool use identified by `tool_use_id`;
    /// MUST reject (never silently no-op) if no such tool use is pending.
    async fn send_tool_result(
        &self,
        session_id: &str,
        tool_use_id: &str,
        result: serde_json::Value,
    ) -> Result<(), AdapterError>;

    /// Reports the session's coarse run status (`running` / `idle` / `error`) —
    /// derived from the same vocabulary as the `status` [`Event`], never
    /// hand-redefined.
    async fn get_session_status(&self, session_id: &str) -> Result<RunStatus, AdapterError>;

    /// Returns a page of the session's normalised event history in emission
    /// order; MUST agree exactly with `stream_events`'s ordering.
    async fn list_session_events(
        &self,
        session_id: &str,
        options: ListSessionEventsOptions,
    ) -> Result<ListSessionEventsResult, AdapterError>;

    /// Streams the session's events in true emission order, terminating once a
    /// `status: idle` or non-recoverable `error` Event has been yielded — and
    /// NOT merely because output paused while `status` is `running`.
    fn stream_events(&self, session_id: &str) -> EventStream;

    /// Enumerates every blocking tool use the session is currently parked on,
    /// each with its correlation id, name and input intact; MUST return an empty
    /// vector (never an error) when nothing is pending.
    async fn get_pending_tool_calls(
        &self,
        session_id: &str,
    ) -> Result<Vec<AdapterPendingToolCall>, AdapterError>;
}
