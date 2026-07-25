//! An in-memory [`MockAgentProvider`] for tests and conformance driving.
//!
//! Where [`MockProvider`](crate::mock::MockProvider) is a canned implementation
//! of the per-turn [`Provider`](crate::Provider) trait, this is a deterministic,
//! in-memory implementation of the OASP [`AgentProvider`] adapter contract — the
//! first implementor of that trait. It is the substrate the OASP conformance kit
//! drives, and it lets downstream crates (the server, later slices) exercise the
//! whole session lifecycle — create agent → create session → send message →
//! stream events → resolve pending tool calls — with **no database and no
//! network**, so its tests run in pure Rust.
//!
//! Available under the `mock` cargo feature (and always within this crate's own
//! tests).
//!
//! # Determinism
//!
//! Ids are minted from monotonic per-provider counters, not random UUIDs or
//! wall-clock values, so a given sequence of calls yields the same ids every
//! run. Session ids are `mock-session-{n}`, provider-agent ids
//! `mock-agent-{n}`, and event ids are 20-digit zero-padded decimals — the same
//! lexicographically-monotonic shape [`Event::id`] uses elsewhere, so string
//! ordering equals emission ordering and doubles as the `list_session_events`
//! cursor. Event `at` timestamps are the one non-deterministic field
//! ([`Utc::now`]); ordering never depends on them, so tests should compare
//! events by `id`/`kind`, not by `at`.
//!
//! # What it models, and what it does not
//!
//! The mock honours the contract's load-bearing invariants: `create_session`
//! echoes the pin, resources and vaults verbatim; `update_agent` mutates in
//! place (never mints a second agent); `send_tool_result` rejects an unknown
//! `tool_use_id` rather than silently no-op'ing; `get_pending_tool_calls`
//! returns an empty vector (never an error) when nothing is pending; and
//! seeding a session with a transcript never fabricates a fresh assistant turn.
//! It does **not** model a real provider's semantics — `send_message` appends a
//! fixed canned assistant turn ending `idle`, and `stream_events` replays the
//! session's current finite event snapshot. Richer scripted behaviour is layered
//! on by tests via the control methods ([`MockAgentProvider::push_event`],
//! [`MockAgentProvider::add_pending_tool_call`],
//! [`MockAgentProvider::induce_failure`]).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use futures::stream;
use uuid::Uuid;

use loom_core::{
    AgentDefinition, AgentVersionRef, Deployment, Event, EventKind, RunStatus, SessionResource,
};

use crate::adapter::{
    AdapterError, AdapterPendingToolCall, AdapterSession, AgentProvider, CreateSessionOptions,
    EnsureEnvironmentResult, EventStream, ListSessionEventsOptions, ListSessionEventsResult,
};

/// The canned assistant reply text `send_message` emits.
const CANNED_REPLY: &str = "mock response";

/// A single in-memory session's mutable state.
#[derive(Clone, Debug)]
struct MockSession {
    pinned_agent_version: AgentVersionRef,
    resources: Vec<SessionResource>,
    vault_ids: Vec<String>,
    /// The session's normalised event log, in emission (== id) order.
    events: Vec<Event>,
    /// The blocking tool uses the session is currently parked on.
    pending: Vec<AdapterPendingToolCall>,
    /// The session's coarse run status.
    status: RunStatus,
    /// Monotonic per-session event sequence, formatted into 20-digit ids.
    event_seq: u64,
}

impl MockSession {
    fn as_adapter(&self, id: String) -> AdapterSession {
        AdapterSession {
            id,
            pinned_agent_version: self.pinned_agent_version,
            resources: self.resources.clone(),
            vault_ids: self.vault_ids.clone(),
        }
    }

    /// Appends `kind` as the next event, minting a fresh monotonic id, and
    /// returns the stored event.
    fn append(&mut self, kind: EventKind) -> Event {
        self.event_seq += 1;
        let event = Event {
            id: format!("{:020}", self.event_seq),
            at: Utc::now(),
            kind,
        };
        self.events.push(event.clone());
        event
    }
}

/// The provider's whole in-memory world, guarded by a single mutex.
#[derive(Debug, Default)]
struct MockState {
    environments: BTreeSet<String>,
    /// `provider_agent_id` → its current deployment.
    agents: BTreeMap<String, Deployment>,
    /// `session_id` → its state.
    sessions: BTreeMap<String, MockSession>,
    /// Operation names currently rigged to fail with an induced error.
    induced: BTreeSet<String>,
    next_agent: u64,
    next_session: u64,
}

/// A deterministic, in-memory [`AgentProvider`] for tests and conformance.
///
/// Clones share one backing store (the state sits behind an [`Arc`]), so a test
/// can hold one handle to drive controls ([`push_event`](Self::push_event),
/// [`induce_failure`](Self::induce_failure), …) while handing a clone to the
/// code under test as a `dyn AgentProvider`.
#[derive(Clone, Debug, Default)]
pub struct MockAgentProvider {
    state: Arc<Mutex<MockState>>,
}

impl MockAgentProvider {
    /// Creates an empty mock provider — no environments, agents or sessions.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Rigs `operation` (the trait method name, e.g. `"create_session"`) to
    /// fail with [`AdapterError::induced_failure`] until
    /// [`clear_induced_failure`](Self::clear_induced_failure) is called. For
    /// `stream_events`, the induced failure surfaces as a single terminal
    /// `error` event rather than an `Err`, honouring that method's stream-only
    /// failure channel.
    pub fn induce_failure(&self, operation: impl Into<String>) {
        self.lock().induced.insert(operation.into());
    }

    /// Clears a previously-rigged induced failure for `operation`; returns
    /// whether one was set.
    pub fn clear_induced_failure(&self, operation: &str) -> bool {
        self.lock().induced.remove(operation)
    }

    /// Appends `kind` to `session_id`'s event log as the next event, returning
    /// the stored event, or `None` if no such session exists. A test control for
    /// scripting event histories the canned `send_message` does not produce
    /// (tool uses, thinking, multi-turn sequences).
    pub fn push_event(&self, session_id: &str, kind: EventKind) -> Option<Event> {
        self.lock()
            .sessions
            .get_mut(session_id)
            .map(|session| session.append(kind))
    }

    /// Registers a pending tool call against `session_id` so
    /// [`get_pending_tool_calls`](AgentProvider::get_pending_tool_calls) and
    /// [`send_tool_result`](AgentProvider::send_tool_result) have something to
    /// correlate against. Returns whether the session existed.
    pub fn add_pending_tool_call(&self, session_id: &str, call: AdapterPendingToolCall) -> bool {
        match self.lock().sessions.get_mut(session_id) {
            Some(session) => {
                session.pending.push(call);
                true
            }
            None => false,
        }
    }

    /// Overrides `session_id`'s coarse run status. Returns whether the session
    /// existed.
    pub fn set_status(&self, session_id: &str, status: RunStatus) -> bool {
        match self.lock().sessions.get_mut(session_id) {
            Some(session) => {
                session.status = status;
                true
            }
            None => false,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, MockState> {
        // A poisoned lock means a prior test panicked mid-mutation; recovering
        // the guard keeps one failure from cascading into every later test.
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Returns an induced-failure error for `operation` if it is currently rigged.
fn check_induced(state: &MockState, operation: &str) -> Result<(), AdapterError> {
    if state.induced.contains(operation) {
        Err(AdapterError::induced_failure(operation))
    } else {
        Ok(())
    }
}

/// The mock's canonical-hash stand-in: a deterministic, human-readable key over
/// the definition's identity and draft version. A real adapter hashes the
/// definition's full canonical form; a mock only needs a stable idempotency key.
fn canonical_hash(definition: &AgentDefinition) -> String {
    format!("mock-hash-{}-{}", definition.id, definition.draft_version)
}

#[async_trait]
impl AgentProvider for MockAgentProvider {
    async fn ensure_environment(
        &self,
        environment_id: &str,
    ) -> Result<EnsureEnvironmentResult, AdapterError> {
        let mut state = self.lock();
        check_induced(&state, "ensure_environment")?;
        // Idempotent: re-inserting an existing environment is a no-op.
        state.environments.insert(environment_id.to_owned());
        Ok(EnsureEnvironmentResult {
            environment_id: environment_id.to_owned(),
        })
    }

    async fn create_agent(
        &self,
        definition: &AgentDefinition,
        environment_id: &str,
    ) -> Result<Deployment, AdapterError> {
        let mut state = self.lock();
        check_induced(&state, "create_agent")?;
        state.next_agent += 1;
        let provider_agent_id = format!("mock-agent-{}", state.next_agent);
        let deployment = Deployment {
            id: Uuid::from_u128(u128::from(state.next_agent)),
            agent_definition_id: definition.id,
            provider: "mock".to_owned(),
            provider_agent_id: provider_agent_id.clone(),
            environment_id: environment_id.to_owned(),
            provider_version: format!("mock-v{}", definition.draft_version),
            canonical_hash: canonical_hash(definition),
        };
        state.agents.insert(provider_agent_id, deployment.clone());
        Ok(deployment)
    }

    async fn update_agent(
        &self,
        provider_agent_id: &str,
        definition: &AgentDefinition,
        environment_id: &str,
    ) -> Result<Deployment, AdapterError> {
        let mut state = self.lock();
        check_induced(&state, "update_agent")?;
        // Update in place — MUST NOT create a second agent. A missing agent is
        // an error, not an implicit create.
        let Some(existing) = state.agents.get(provider_agent_id) else {
            return Err(AdapterError::agent_not_found(provider_agent_id));
        };
        let deployment = Deployment {
            id: existing.id,
            agent_definition_id: definition.id,
            provider: "mock".to_owned(),
            provider_agent_id: provider_agent_id.to_owned(),
            environment_id: environment_id.to_owned(),
            provider_version: format!("mock-v{}", definition.draft_version),
            canonical_hash: canonical_hash(definition),
        };
        state
            .agents
            .insert(provider_agent_id.to_owned(), deployment.clone());
        Ok(deployment)
    }

    async fn get_agent(&self, provider_agent_id: &str) -> Result<Deployment, AdapterError> {
        let state = self.lock();
        check_induced(&state, "get_agent")?;
        state
            .agents
            .get(provider_agent_id)
            .cloned()
            .ok_or_else(|| AdapterError::agent_not_found(provider_agent_id))
    }

    async fn create_session(
        &self,
        options: CreateSessionOptions,
    ) -> Result<AdapterSession, AdapterError> {
        let mut state = self.lock();
        check_induced(&state, "create_session")?;
        state.next_session += 1;
        let session_id = format!("mock-session-{}", state.next_session);

        let mut session = MockSession {
            pinned_agent_version: options.pinned_agent_version,
            resources: options.resources,
            vault_ids: options.vault_ids,
            events: Vec::new(),
            pending: Vec::new(),
            status: RunStatus::Idle,
            event_seq: 0,
        };

        // Seed transcript (migrate's Stage 2): the events are ingested as
        // already-exchanged history under fresh monotonic ids, and — crucially —
        // NO fresh assistant turn is fabricated in response to the seed alone.
        if let Some(seed) = options.seed {
            for event in seed.events {
                session.append(event.kind);
            }
        }

        let adapter_session = session.as_adapter(session_id.clone());
        state.sessions.insert(session_id, session);
        Ok(adapter_session)
    }

    async fn send_message(
        &self,
        session_id: &str,
        _content: &str,
        _principal: Option<&str>,
    ) -> Result<(), AdapterError> {
        let mut state = self.lock();
        check_induced(&state, "send_message")?;
        let Some(session) = state.sessions.get_mut(session_id) else {
            return Err(AdapterError::session_not_found(session_id));
        };
        // Canned turn: a single assistant message, settling back to idle.
        let message_id = format!("mock-msg-{}", session.event_seq + 1);
        session.append(EventKind::AssistantMessageStart {
            message_id: message_id.clone(),
        });
        session.append(EventKind::AssistantMessageText {
            message_id: message_id.clone(),
            delta: CANNED_REPLY.to_owned(),
        });
        session.append(EventKind::AssistantMessageEnd { message_id });
        session.append(EventKind::Status {
            status: RunStatus::Idle,
        });
        session.status = RunStatus::Idle;
        Ok(())
    }

    async fn send_tool_result(
        &self,
        session_id: &str,
        tool_use_id: &str,
        _result: serde_json::Value,
    ) -> Result<(), AdapterError> {
        let mut state = self.lock();
        check_induced(&state, "send_tool_result")?;
        let Some(session) = state.sessions.get_mut(session_id) else {
            return Err(AdapterError::session_not_found(session_id));
        };
        // Correlate against the pending set — reject (never silently no-op) a
        // tool_use_id with no matching pending call.
        let before = session.pending.len();
        session
            .pending
            .retain(|call| call.tool_use_id != tool_use_id);
        if session.pending.len() == before {
            return Err(AdapterError::unknown_tool_use(tool_use_id));
        }
        Ok(())
    }

    async fn get_session_status(&self, session_id: &str) -> Result<RunStatus, AdapterError> {
        let state = self.lock();
        check_induced(&state, "get_session_status")?;
        state
            .sessions
            .get(session_id)
            .map(|session| session.status)
            .ok_or_else(|| AdapterError::session_not_found(session_id))
    }

    async fn list_session_events(
        &self,
        session_id: &str,
        options: ListSessionEventsOptions,
    ) -> Result<ListSessionEventsResult, AdapterError> {
        let state = self.lock();
        check_induced(&state, "list_session_events")?;
        let Some(session) = state.sessions.get(session_id) else {
            return Err(AdapterError::session_not_found(session_id));
        };

        // Events strictly after `after_id`, ordered by id (already the storage
        // order). A negative limit is treated as zero.
        let limit = options
            .limit
            .map_or(usize::MAX, |limit| usize::try_from(limit).unwrap_or(0));
        let matching: Vec<&Event> = session
            .events
            .iter()
            .filter(|event| {
                options
                    .after_id
                    .as_deref()
                    .is_none_or(|after| event.id.as_str() > after)
            })
            .collect();
        let more = matching.len() > limit;
        let page: Vec<Event> = matching.into_iter().take(limit).cloned().collect();
        // A cursor only when more events remain beyond this page; `None` once the
        // page reaches the end of the currently-known history.
        let next_cursor = if more {
            page.last().map(|event| event.id.clone())
        } else {
            None
        };
        Ok(ListSessionEventsResult {
            events: page,
            next_cursor,
        })
    }

    fn stream_events(&self, session_id: &str) -> EventStream {
        let state = self.lock();
        let events = match state.sessions.get(session_id) {
            // A finite snapshot of the log — its finiteness IS its termination.
            Some(session) => session.events.clone(),
            // No `Result` channel here: an unknown session surfaces as a single
            // terminal, non-recoverable `error` event.
            None => vec![Event {
                id: format!("{:020}", 0),
                at: Utc::now(),
                kind: EventKind::Error {
                    message: format!("No session found with id \"{session_id}\"."),
                    recoverable: false,
                },
            }],
        };
        Box::pin(stream::iter(events))
    }

    async fn get_pending_tool_calls(
        &self,
        session_id: &str,
    ) -> Result<Vec<AdapterPendingToolCall>, AdapterError> {
        let state = self.lock();
        check_induced(&state, "get_pending_tool_calls")?;
        let Some(session) = state.sessions.get(session_id) else {
            return Err(AdapterError::session_not_found(session_id));
        };
        // Empty vector (never an error) when nothing is pending.
        Ok(session.pending.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::SeedTranscript;
    use futures::StreamExt;

    fn definition() -> AgentDefinition {
        let now = Utc::now();
        AgentDefinition {
            id: Uuid::from_u128(0x0a11),
            tenant_id: Uuid::from_u128(0x7e11),
            name: "test-agent".to_owned(),
            draft_version: 1,
            published_version: Some(1),
            created_at: now,
            updated_at: now,
        }
    }

    fn version_ref() -> AgentVersionRef {
        AgentVersionRef {
            agent_definition_id: Uuid::from_u128(0x0a11),
            version: 1,
        }
    }

    /// Bare options for `create_session` with no resources, vaults or seed.
    fn create_options() -> CreateSessionOptions {
        CreateSessionOptions {
            agent_definition_id: Uuid::from_u128(0x0a11),
            provider_agent_id: "mock-agent-1".to_owned(),
            pinned_agent_version: version_ref(),
            resources: Vec::new(),
            vault_ids: Vec::new(),
            seed: None,
        }
    }

    async fn new_session(provider: &MockAgentProvider) -> String {
        provider
            .create_session(create_options())
            .await
            .expect("create_session")
            .id
    }

    #[tokio::test]
    async fn create_session_echoes_pin_resources_vaults() {
        let provider = MockAgentProvider::new();
        let resources = vec![
            SessionResource::File {
                file_id: "f-1".to_owned(),
            },
            SessionResource::GitHubRepository {
                owner: "fieldstatenz".to_owned(),
                repo: "loom".to_owned(),
                r#ref: Some("main".to_owned()),
            },
        ];
        let vault_ids = vec!["vault-a".to_owned(), "vault-b".to_owned()];
        let options = CreateSessionOptions {
            resources: resources.clone(),
            vault_ids: vault_ids.clone(),
            ..create_options()
        };

        let session = provider.create_session(options).await.expect("create");
        // The pin, resources and vaults come back verbatim, in order.
        assert_eq!(session.pinned_agent_version, version_ref());
        assert_eq!(session.resources, resources);
        assert_eq!(session.vault_ids, vault_ids);
        // A brand-new session is idle with no events yet.
        assert_eq!(
            provider.get_session_status(&session.id).await.unwrap(),
            RunStatus::Idle
        );
        let listed = provider
            .list_session_events(&session.id, ListSessionEventsOptions::default())
            .await
            .unwrap();
        assert!(listed.events.is_empty());
        assert!(listed.next_cursor.is_none());
    }

    #[tokio::test]
    async fn ensure_environment_is_idempotent() {
        let provider = MockAgentProvider::new();
        let first = provider.ensure_environment("prod").await.unwrap();
        let second = provider.ensure_environment("prod").await.unwrap();
        assert_eq!(first.environment_id, "prod");
        assert_eq!(second.environment_id, "prod");
    }

    #[tokio::test]
    async fn update_agent_mutates_in_place() {
        let provider = MockAgentProvider::new();
        let created = provider
            .create_agent(&definition(), "prod")
            .await
            .expect("create_agent");
        assert_eq!(created.provider_agent_id, "mock-agent-1");
        assert_eq!(created.provider_version, "mock-v1");

        let mut next = definition();
        next.draft_version = 2;
        let updated = provider
            .update_agent(&created.provider_agent_id, &next, "prod")
            .await
            .expect("update_agent");
        // Same deployment id and provider-agent id: mutated in place.
        assert_eq!(updated.id, created.id);
        assert_eq!(updated.provider_agent_id, created.provider_agent_id);
        assert_eq!(updated.provider_version, "mock-v2");
        // get_agent reflects the update.
        assert_eq!(
            provider
                .get_agent(&created.provider_agent_id)
                .await
                .unwrap(),
            updated
        );
        // No second agent was minted: `mock-agent-2` does not exist.
        assert_eq!(
            provider.get_agent("mock-agent-2").await.unwrap_err().code,
            "Adapter.AgentNotFound"
        );
    }

    #[tokio::test]
    async fn update_or_get_unknown_agent_is_error() {
        let provider = MockAgentProvider::new();
        assert_eq!(
            provider
                .update_agent("ghost", &definition(), "prod")
                .await
                .unwrap_err()
                .code,
            "Adapter.AgentNotFound"
        );
        assert_eq!(
            provider.get_agent("ghost").await.unwrap_err().code,
            "Adapter.AgentNotFound"
        );
    }

    #[tokio::test]
    async fn send_message_appends_canned_turn_in_order() {
        let provider = MockAgentProvider::new();
        let session_id = new_session(&provider).await;
        provider
            .send_message(&session_id, "hello", Some("user-1"))
            .await
            .expect("send_message");

        let listed = provider
            .list_session_events(&session_id, ListSessionEventsOptions::default())
            .await
            .unwrap();
        let kinds: Vec<&EventKind> = listed.events.iter().map(|e| &e.kind).collect();
        assert!(matches!(
            kinds.as_slice(),
            [
                EventKind::AssistantMessageStart { .. },
                EventKind::AssistantMessageText { delta, .. },
                EventKind::AssistantMessageEnd { .. },
                EventKind::Status { status: RunStatus::Idle },
            ] if delta == CANNED_REPLY
        ));
        // Ids are strictly increasing in emission order.
        let ids: Vec<&str> = listed.events.iter().map(|e| e.id.as_str()).collect();
        assert!(ids.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(
            provider.get_session_status(&session_id).await.unwrap(),
            RunStatus::Idle
        );
    }

    #[tokio::test]
    async fn list_session_events_paginates_by_cursor() {
        let provider = MockAgentProvider::new();
        let session_id = new_session(&provider).await;
        provider
            .send_message(&session_id, "hi", None)
            .await
            .unwrap(); // 4 events

        let page1 = provider
            .list_session_events(
                &session_id,
                ListSessionEventsOptions {
                    after_id: None,
                    limit: Some(2),
                },
            )
            .await
            .unwrap();
        assert_eq!(page1.events.len(), 2);
        let cursor = page1.next_cursor.clone().expect("more events remain");
        assert_eq!(cursor, page1.events[1].id);

        let page2 = provider
            .list_session_events(
                &session_id,
                ListSessionEventsOptions {
                    after_id: Some(cursor),
                    limit: Some(2),
                },
            )
            .await
            .unwrap();
        assert_eq!(page2.events.len(), 2);
        // Reached the end: no cursor, and the two pages are disjoint and ordered.
        assert!(page2.next_cursor.is_none());
        assert!(page1.events[1].id < page2.events[0].id);
    }

    #[tokio::test]
    async fn send_tool_result_correlates_and_rejects_unknown() {
        let provider = MockAgentProvider::new();
        let session_id = new_session(&provider).await;
        assert!(provider.add_pending_tool_call(
            &session_id,
            AdapterPendingToolCall {
                tool_use_id: "call-1".to_owned(),
                name: "get_weather".to_owned(),
                input: serde_json::json!({ "city": "Wellington" }),
                mcp_server_url: None,
            },
        ));
        // Enumerated before resolution.
        assert_eq!(
            provider
                .get_pending_tool_calls(&session_id)
                .await
                .unwrap()
                .len(),
            1
        );
        // Resolving a known call clears it.
        provider
            .send_tool_result(&session_id, "call-1", serde_json::json!({ "temp_c": 12 }))
            .await
            .expect("resolve known call");
        assert!(provider
            .get_pending_tool_calls(&session_id)
            .await
            .unwrap()
            .is_empty());
        // Resolving it again — now unknown — is rejected, never a silent no-op.
        assert_eq!(
            provider
                .send_tool_result(&session_id, "call-1", serde_json::Value::Null)
                .await
                .unwrap_err()
                .code,
            "Adapter.UnknownToolUse"
        );
        // An id that never existed is likewise rejected.
        assert_eq!(
            provider
                .send_tool_result(&session_id, "never", serde_json::Value::Null)
                .await
                .unwrap_err()
                .code,
            "Adapter.UnknownToolUse"
        );
    }

    #[tokio::test]
    async fn get_pending_tool_calls_is_empty_not_error_when_none() {
        let provider = MockAgentProvider::new();
        let session_id = new_session(&provider).await;
        assert!(provider
            .get_pending_tool_calls(&session_id)
            .await
            .expect("empty is Ok, not Err")
            .is_empty());
    }

    #[tokio::test]
    async fn unknown_session_is_session_not_found() {
        let provider = MockAgentProvider::new();
        for code in [
            provider
                .send_message("ghost", "x", None)
                .await
                .unwrap_err()
                .code,
            provider.get_session_status("ghost").await.unwrap_err().code,
            provider
                .list_session_events("ghost", ListSessionEventsOptions::default())
                .await
                .unwrap_err()
                .code,
            provider
                .get_pending_tool_calls("ghost")
                .await
                .unwrap_err()
                .code,
            provider
                .send_tool_result("ghost", "t", serde_json::Value::Null)
                .await
                .unwrap_err()
                .code,
        ] {
            assert_eq!(code, "Adapter.SessionNotFound");
        }
    }

    #[tokio::test]
    async fn seed_transcript_does_not_fabricate_a_fresh_turn() {
        let provider = MockAgentProvider::new();
        // A settled prior history: one status event, already exchanged.
        let seed = SeedTranscript {
            events: vec![Event {
                id: "seed-id-ignored".to_owned(),
                at: Utc::now(),
                kind: EventKind::Status {
                    status: RunStatus::Idle,
                },
            }],
        };
        let options = CreateSessionOptions {
            seed: Some(seed),
            ..create_options()
        };
        let session = provider.create_session(options).await.unwrap();

        let listed = provider
            .list_session_events(&session.id, ListSessionEventsOptions::default())
            .await
            .unwrap();
        // Exactly the seeded event is present (re-keyed under a mock id) — the
        // seed produced NO fresh assistant turn.
        assert_eq!(listed.events.len(), 1);
        assert!(matches!(listed.events[0].kind, EventKind::Status { .. }));
        assert!(!listed
            .events
            .iter()
            .any(|e| matches!(e.kind, EventKind::AssistantMessageStart { .. })));
        assert_eq!(
            provider.get_session_status(&session.id).await.unwrap(),
            RunStatus::Idle
        );
    }

    #[tokio::test]
    async fn induced_failure_is_returned_then_cleared() {
        let provider = MockAgentProvider::new();
        provider.induce_failure("create_session");
        let err = provider.create_session(create_options()).await.unwrap_err();
        assert_eq!(err.code, "Adapter.InducedFailure");
        assert!(err.retryable);

        assert!(provider.clear_induced_failure("create_session"));
        provider
            .create_session(create_options())
            .await
            .expect("succeeds once the induced failure is cleared");
    }

    #[tokio::test]
    async fn stream_events_replays_snapshot_and_errors_on_unknown() {
        let provider = MockAgentProvider::new();
        let session_id = new_session(&provider).await;
        provider
            .send_message(&session_id, "hi", None)
            .await
            .unwrap();

        let streamed: Vec<Event> = provider.stream_events(&session_id).collect().await;
        let listed = provider
            .list_session_events(&session_id, ListSessionEventsOptions::default())
            .await
            .unwrap();
        // The stream is exactly the current log snapshot, in order.
        assert_eq!(streamed, listed.events);

        // An unknown session yields a single terminal, non-recoverable error.
        let ghost: Vec<Event> = provider.stream_events("ghost").collect().await;
        assert_eq!(ghost.len(), 1);
        assert!(matches!(
            ghost[0].kind,
            EventKind::Error {
                recoverable: false,
                ..
            }
        ));
    }
}
