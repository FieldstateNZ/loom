# Loom ↔ OASP contract — gap analysis and forward roadmap

**Status:** analysis complete. This is the audit called for in
[loom#30](https://github.com/fieldstatenz/loom/issues/30). It inventories what
Loom has built against the OASP managed-agent contract it implements, and turns
the gaps into a prioritised roadmap.

**Read this alongside** [`concepts/loom-and-oasp.md`](./concepts/loom-and-oasp.md)
(the design thesis). The framing matters: Loom is the product; OASP is the
contract it stands on. So the gaps below are **"what Loom still needs to build to
hold its own managed-agent contract over any provider"** — not "distance from a
spec." A durable Session, a memory layer, an audit plane are things Loom must own
precisely so it can *be* the managed agent over a provider that only does
completion.

## Target conformance

OASP defines three composing conformance levels (concept draft § Conformance;
[`docs/spec/README.md`](https://github.com/oasp-dev/oasp-standard/blob/main/docs/spec/README.md)):

| Level | Meaning | Loom's stance |
| --- | --- | --- |
| **Level 1 — Client** | Consumes the API + event vocabulary | Effectively met by the `@fieldstate/loom-client` TS SDK once the server emits the normalised vocabulary. |
| **Level 2 — Server** | Implements the resources + the 7 interactions | **Primary target.** The bulk of the roadmap below. |
| **Level 3 — Adapter** | Maps a provider preserving required semantics | **Target for Anthropic** — the reference adapter. A server may not claim Level 3 for a provider until it has passed the kit against it. |

Loom targets **Level 2 (Server) + Level 3 (Anthropic Adapter)**. The executable
conformance kit ([`packages/conformance`](https://github.com/oasp-dev/oasp-standard/tree/main/packages/conformance))
checks all three deterministically against a mock — no live keys, CI-safe. Live
provider behaviour is confirmed separately by the per-adapter live-smoke ritual
(see the roadmap's OASP-11).

## Where Loom stands today (the honest summary)

Loom's durable unit is a `Conversation` — a tenant-scoped, `{provider, model}`-bound,
mutable `Message` list — and its verb is a **turn**. Measured against the
contract, that means **Loom's `Conversation` currently occupies the role of OASP's
`Session`**, and OASP's durable `Conversation` (a stable identity riding a lineage
of Sessions) plus the whole Session-lifecycle / agent-version / identity / audit
model has no representation yet.

- **Interactions:** of the 7, `send` exists (divergent — it fuses send + execute +
  respond); `createConversation`, `stream`, and `sendToolResult` are partial;
  `publish`, `migrate`, `drain` are missing.
- **Adapter:** Loom's `Provider` trait has 4 methods; OASP's `AgentProvider` has
  10. **0 of the 8 MUST-preserve invariants** are fully satisfied today.
- **Philosophy note (not a defect):** Loom is *lossless-passthrough* today — every
  event carries a verbatim `raw`, and unknown blocks route to
  `ProviderExtension`. The contract wants a *normalised, closed, id-bearing* event
  stream. Loom over-preserves relative to the contract; it just doesn't yet
  *produce* the normalised stream conformance checks against. Keeping `raw` as an
  escape hatch remains a feature — it is how no capability is dropped.

### Resource model

*(Evidence file:line references are a snapshot at time of analysis.)*

| OASP resource | Loom status | Notes |
| --- | --- | --- |
| **AgentDefinition** | MISSING | Loom pins `{provider, model}` strings (`loom-core .../provider_binding.rs`); no named, versioned, provider-neutral agent. System prompt lives on the conversation; tools are per-request. |
| **AgentDefinitionVersion** | MISSING | No agent versioning. The only append-only versioned entity is `model_prices` (pricing, orthogonal). |
| **Deployment** | MISSING | No provider-side agent materialisation / idempotent deploy. Loom talks to providers per-request. |
| **Conversation** | PARTIAL | Durable + first-class (`conversations` table), but **inverted**: owns its `Message` history directly and binds to a model. No `currentSessionId`, `previousSessionIds`, `pinnedAgentVersion`, `initiatingPrincipal`, or `scope`. Ownership is a flat `tenant_id`, not the 5-level scope taxonomy. |
| **Session** | **MISSING** | The root gap. No durable provider execution context; execution is per-turn. No `resources[]`, `vaultIds[]`, or lifecycle. |
| **Event** | MISSING (as a resource) | Streams SSE deltas transiently, persists whole `Message` rows. No typed, order-cursored event log for `listSessionEvents`. |
| **Principal** | MISSING | Identity is `Tenant` + `VirtualKey` (billing-shaped). No OIDC-mappable actor, `kind`, `roles`, or `scopeMemberships`. |
| **AuditEvent** | MISSING | `usage_events` / `usage_outbox` record spend, not who/what/when/outcome. |
| **Credential** | PARTIAL | Stores the encrypted secret **at rest**, not a provider **vault reference**; no scope-pin / `onBehalfOf`; MCP auth lives in a separate name-keyed table. |

### Interactions

| OASP interaction | Loom status | Nearest surface | Gap |
| --- | --- | --- | --- |
| `publish` | MISSING | — | No AgentDefinition / draft-published pointers to publish. |
| `createConversation` | PARTIAL | `POST /v1/conversations` | Mints a model-bound Conversation; does not mint a Session, pin a version, mount `resources[]`, or resolve `vaultIds[]`. |
| `migrate` | MISSING | — | No Session, no lineage, no transcript-seed / atomic swap. The crown-jewel mechanism is absent. |
| `drain` | MISSING | — | No "parked on pending tool calls" state and no recovery path. |
| `stream` | PARTIAL | `POST .../turns?stream=true` (SSE) | Per-turn POST stream, not a session-scoped, id-addressable, replayable event log. No `GET .../events`, no `listSessionEvents`, no per-`Event` id, not the closed vocabulary. |
| `send` | EXISTS (divergent) | `POST /v1/conversations/{id}/turns` | Posts + persists a caller turn and runs the provider. Targets a Conversation, not a Session; fuses send + execute + respond; no `currentSessionId` guard. |
| `sendToolResult` | PARTIAL (as convention) | inline `ContentPart::ToolResult` | Round-trips losslessly, but there is no pending-tool-call set to correlate a `toolUseId` against, and no rejection of unknown/duplicate ids. |

### Adapter contract (Level 3)

Loom's `Provider` trait (`descriptor` / `complete` / `stream` / `count_cost`) is a
per-turn interface; OASP's `AgentProvider` is a 10-op session-lifecycle interface
(`ensureEnvironment`, `createAgent` / `updateAgent` / `getAgent`, `createSession`,
`sendMessage`, `sendToolResult`, `getSessionStatus`, `listSessionEvents`,
`streamEvents`, `getPendingToolCalls`).

The 8 MUST-preserve invariants — **none fully satisfied today**: version pinning
(no `AgentVersionRef`); resource + credential fidelity at session creation (no
`createSession`); pending-tool-call enumeration (no `getPendingToolCalls`);
event-id ordering (`TurnEvent` has no `id`); the closed 8-variant vocabulary
(Loom's `TurnEventKind` is a different, `#[non_exhaustive]` delta-centric set with
no `status`/`error` event variants); tool-result correlation; `status: idle`
termination (no running/idle model); no fresh turn from seeding (no seed concept).

The Anthropic provider is a **strong Messages translator** — streaming +
non-streaming translation, prompt caching, server tools, MCP connector with
server-side token injection, batches — and a credible basis for the reference
adapter. It is not yet an OASP *adapter*: it implements the 4-method turn trait,
with no session lifecycle, agent materialisation, version pinning, pending-call
enumeration, or id-bearing normalised stream.

## Forward roadmap

Dependency-ordered, grouped into conformance-gated waves. Each item is filed as a
Loom issue cross-linked to its OASP spec section; every wave is gated by
`cargo fmt`/`clippy`/`test`, and from Wave 5 by the OASP conformance kit
(self-report, no live keys). Priorities: **P0** foundational, **P1** contract-core,
**P2** hardening.

**Wave 1 — the substrate**
- **OASP-1 [P0] Conversation ≠ Session split.** Introduce `Session` as a
  first-class durable resource; re-model `Conversation` to own a lineage of
  Sessions (`currentSessionId`, `previousSessionIds`) instead of owning messages
  directly; move message/event ownership under Session. Done **additively** so the
  existing `/v1` surface keeps working. *(spec: `conversation-and-session.md`)*

**Wave 2 — agent model + event log** *(parallel after W1)*
- **OASP-2 [P0] AgentDefinition + AgentDefinitionVersion + Deployment + `publish`.**
  Named, provider-neutral, versioned agent; draft/published pointers; immutable
  version snapshots; deployment materialisation; the version-pinning hook.
  *(spec: `interactions.md`, `target-version-resolution.md`)*
- **OASP-3 [P1] Normalised Event vocabulary + session event log +
  `stream`/`listSessionEvents`.** The closed 8-variant `Event` with
  lexicographically-monotonic ids; a durable per-session log; `GET /sessions/{id}/events`
  (SSE) + cursor-paginated `listSessionEvents`. *(spec: `interactions.md`, `adapters.md`)*

**Wave 3 — tool lifecycle + the adapter** *(after W2)*
- **OASP-4 [P1] Pending-tool-call model + `sendToolResult` + `drain`.** Server-side
  pending set; `getPendingToolCalls`; `sendToolResult` correlation/rejection;
  `drain` with per-call authorisation against the pinned grants + idle
  confirmation. *(spec: `interactions.md`)*
- **OASP-9 [P1] Anthropic OASP adapter (`AgentProvider` 10-op contract).** Refactor
  the 4-method `Provider` trait to the session-lifecycle `AgentProvider`; implement
  the real Anthropic adapter (createAgent/session, sendMessage, getPendingToolCalls,
  streamEvents → 8-variant vocab, version pinning, resource/vault fidelity).
  *(spec: `adapters.md`)*

**Wave 4 — migration**
- **OASP-5 [P1] `migrate` + session lifecycle.** The 4-stage migrate (mint at
  resolved target, transcript-seed with suppression marker, drain to idle, atomic
  swap + lineage append), degrade-to-fresh-start, per-conversation serialisation.
  *(spec: `interactions.md`, `target-version-resolution.md`)*

**Cross-cutting — identity / audit / credential** *(paced against the still-hardening standard surfaces; see below)*
- **OASP-6 [P1] Identity plane: Principal + Scope taxonomy + on-behalf-of
  containment.** *(spec: `scope-and-identity.md`; tracks `oasp-standard#7`)*
- **OASP-7 [P1] AuditEvent emission** — the required-emission set (one per
  interaction incl. `stream` + not-found), the minimum shape, derive-on-read
  source. *(spec: `audit.md`; tracks `oasp-standard#11`)*
- **OASP-8 [P2] Credential remodel** — vault-reference model, URL-matched
  resolution at session creation, scope-pin + `onBehalfOf`; fold MCP-server auth
  into it. *(spec: `adapters.md`, `scope-and-identity.md`; tracks
  `oasp-standard#8`, `#16`)*

**Wave 5 — conformance + release ritual**
- **OASP-10 [P1] Conformance-kit integration + self-report Level 2 + Level 3.** Wire
  `packages/conformance` against Loom (SDK-against-mock, CI-safe); self-report +
  verify. The gate on the whole program.
- **OASP-11 [P2] Anthropic-adapter live-smoke ritual** *(re-scopes
  [loom#27](https://github.com/fieldstatenz/loom/issues/27))* — the 5-step release
  ritual (happy path · stream · tool use · induced error · context overflow)
  against real credentials on hosted staging; on release, never in CI. Depends on
  OASP-9 + staging.

**Epic**
- **LucidBrain becomes Loom consumer #1** — swap LucidBrain's in-repo
  `managed-agents` library for the Loom/OASP client. Tracked; not built here.

## A note on the still-hardening standard surfaces

The identity/audit/credential surfaces of OASP are actively being specified in the
standard repo (`oasp-standard` #7 authenticated actor, #8 credential binding, #11
audit-to-identity, #16 credential lifecycle). Loom implements the **stable core**
first (the Conversation/Session split, agent/version model, event log, tool
lifecycle, migrate, the Anthropic adapter) and paces OASP-6/7/8 against those
issues settling, so Loom doesn't chase a moving target.
