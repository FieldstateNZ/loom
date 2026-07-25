# Conversation and Session

> See [`docs/oasp-v0-concept.md` § The one structural insight the standard is built on](../oasp-v0-concept.md#the-one-structural-insight-the-standard-is-built-on).

**A Conversation is not a Session.** Every interaction specified in
[`interactions.md`](./interactions.md) — `publish`, `createConversation`,
`migrate`, `drain`, `stream`, `send`, `sendToolResult` — is a
consequence of this one split. This document specifies the two
resources and the invariants that hold between them; it does not
itself specify the interactions that maintain those invariants (that's
[`interactions.md`](./interactions.md)).

## The insight

> **Note (non-normative framing):** On a loom, the *warp* is held under
> tension across the frame for the life of the weave, while the *weft*
> is the thread that comes and goes, pass by pass. A `Conversation` is
> the warp: the durable, user-facing thread that outlives any single
> execution context. A `Session` is the weft: a disposable, replaceable,
> provider-side execution context that rides on the Conversation for a
> while and is then superseded. This is where the "Session" name comes
> from and why it was kept over alternatives — see
> [`docs/oasp-v0-concept.md` § Decisions taken](../oasp-v0-concept.md#decisions-taken).
> No provider API models this split; every serious agent application
> hand-builds it. That is the gap OASP fills.

No requirement in this document depends on the loom metaphor — it is
here to make the shape memorable, not to define it. The normative
content is the resource shapes and invariants below.

## `Conversation`: the durable thread

Defined by
[`conversationSchema`](../../packages/schemas/src/resources/conversation.ts)
/ [`Conversation.json`](../../schemas/v1alpha1/Conversation.json).
Fields relevant to this document:

| Field | Meaning |
|---|---|
| `currentSessionId` | Identifier of the `Session` this Conversation presently rides on. |
| `pinnedAgentVersion` | The immutable `AgentDefinition` version the current session was minted against. |
| `previousSessionIds` | Identifiers of every `Session` this Conversation rode on before the current one, oldest first — the lineage `migrate` appends to. |
| `scope` / `initiatingPrincipal` | Generalized-ownership attachment and the `Principal` that started the Conversation. Full normative treatment is S2 ([#3](https://github.com/FieldstateNZ/oasp-standard/issues/3)); not repeated here. |

Normative invariants a conformant server **MUST** maintain for every
`Conversation`:

- A Conversation **MUST** have exactly one `currentSessionId` at any
  point in time, naming the one `Session` it presently rides on. A
  Conversation with no live session (e.g. mid-`migrate`, before the
  atomic swap completes) is not externally observable in that state —
  see [`interactions.md` § Stage 4 — Atomic swap + lineage append](./interactions.md#stage-4--atomic-swap--lineage-append).
- A Conversation's `pinnedAgentVersion` **MUST** always equal the
  `pinnedAgentVersion` of the `Session` identified by its
  `currentSessionId`. This isn't an incidental consistency rule to
  enforce after the fact — per `conversationSchema`'s own field
  description ("the immutable AgentDefinition version *the current
  session was minted against*"), the two are definitionally the same
  value. A conformant server **MUST** update both fields together, and
  the only interaction that is permitted to do so is `migrate`'s
  atomic swap.
- `previousSessionIds` **MUST** be append-only and oldest-first: see
  [The lineage](#the-lineage-previoussessionids) below.

## `Session`: the disposable execution context

Defined by
[`sessionSchema`](../../packages/schemas/src/resources/session.ts) /
[`Session.json`](../../schemas/v1alpha1/Session.json). A Session is
created **pinned to one agent version**, with its resources mounted
and its credentials attached — and, per the schema's own framing,
**carries nothing forward from there**.

| Field | Meaning |
|---|---|
| `pinnedAgentVersion` | The immutable `AgentDefinition` version (`{ agentDefinitionId, version }`, see [`agentVersionRefSchema`](../../packages/schemas/src/common/agent-version-ref.ts)) this Session was created against. |
| `resources` | Array of mounted resources, discriminated on `type`: `file` (`fileId`), `memory_store` (`storeId`, deliberately opaque per [`docs/oasp-v0-concept.md` § Decisions taken](../oasp-v0-concept.md#decisions-taken)), or `github_repository` (`owner`, `repo`, optional `ref`). |
| `vaultIds` | Identifiers of the `Credential`s attached to this Session, matched to MCP servers by URL at creation. |

Normative invariants a conformant server **MUST** maintain for every
`Session`:

- A Session's `pinnedAgentVersion`, `resources`, and `vaultIds`
  **MUST NOT** change after creation. There is no "remount" or
  "repin" operation on an existing Session — obtaining a different
  agent version, resource set, or credential set for a Conversation
  always means minting a *new* Session via `migrate` (see
  [`interactions.md` § Stage 1 — Mint session at target version](./interactions.md#stage-1--mint-session-at-target-version)),
  never mutating the old one in place.
- `vaultIds` **MUST** reference `Credential`s resolved by matching
  each `mcp`-type tool grant's `serverUrl` (on the pinned
  `AgentDefinition` version) against a `Credential`'s `mcpServerUrl`
  (see
  [`credentialSchema`](../../packages/schemas/src/resources/credential.ts)),
  at session-creation time. An `AgentDefinition` **MUST NOT** embed
  credential material directly — an `mcp` tool grant only ever
  declares `auth: 'credential'` as a requirement, never the credential
  itself (see
  [`agentDefinitionSchema`](../../packages/schemas/src/resources/agent-definition.ts)).

> **Note:** A Session's *runtime* status (`running` / `idle` / `error`)
> and its pending tool calls are not fields on the `Session` resource
> itself — they are observed via the Adapter contract's
> `getSessionStatus` / `getPendingToolCalls` operations and reported
> through the [`Event`](./interactions.md#stream) vocabulary. See
> [`docs/oasp-v0-concept.md` § Adapter contract](../oasp-v0-concept.md#adapter-contract).

## The lineage: `previousSessionIds`

The append-only history of every Session a Conversation has ridden on
is what makes the Conversation/Session split auditable rather than
merely convenient. Normative requirements:

- Each time `migrate` swaps a Conversation onto a new Session, the
  server **MUST** append exactly the outgoing `currentSessionId` to
  the end of `previousSessionIds`.
- A server **MUST NOT** reorder `previousSessionIds`, and **MUST NOT**
  remove or rewrite any existing entry in it, in the course of any
  interaction. The list is oldest-first and monotonically growing.
- A server **MUST NOT** delete a `Session` resource while its id still
  appears in any Conversation's `currentSessionId` or
  `previousSessionIds` — retention/garbage-collection policy for
  superseded Sessions beyond that floor is implementation-defined.

Together with the `pinnedAgentVersion` invariant above, this is enough
to answer, for any point in a Conversation's history, exactly which
agent version produced which turns — the foundation the `AuditEvent`
normative spec (S2, [#3](https://github.com/FieldstateNZ/oasp-standard/issues/3))
builds on.

That answer is only as good as what `pinnedAgentVersion` actually
resolves to, though. A bare `{ agentDefinitionId, version }` pointer
(see
[`agentVersionRefSchema`](../../packages/schemas/src/common/agent-version-ref.ts))
names WHICH version produced a turn, but on its own says nothing about
WHAT that version's instructions, tools, or guardrails actually were —
and `AgentDefinition` stores only its CURRENT, still-mutable content,
not a per-version history. Two different `pinnedAgentVersion` values
are trivially distinguishable as integers; recovering what either one
*meant*, content-wise, requires a version to be independently
addressable as immutable content, not merely as a comparable pointer
(issue [#10](https://github.com/FieldstateNZ/oasp-standard/issues/10)).
The
[`AgentDefinitionVersion`](../../packages/schemas/src/resources/agent-definition-version.ts)
resource is that mechanism: a conformant server records one immutable
content snapshot — instructions, provider, model, tools, guardrails —
per `{ agentDefinitionId, version }` pair, at the moment that version
number is minted (whether or not it is ever published), and every
credential/tool-grant resolution that must act "as of" a pinned
version reads from that snapshot rather than from the live
`AgentDefinition`. Without it, "exactly which agent version produced
which turns" answers only "an integer," not "what that integer's
content was" — this document's own claim above would otherwise
overreach the data model backing it. A cryptographic canonicalization
or content-hash scheme for that snapshot is explicitly out of scope
here — issue [#18](https://github.com/FieldstateNZ/oasp-standard/issues/18);
the snapshot's identity is its `{ agentDefinitionId, version }` key
alone.

## Out of scope here

- **Group (multi-agent) conversations** are a v0.1 extension per
  [`docs/oasp-v0-concept.md` § Decisions taken](../oasp-v0-concept.md#decisions-taken).
  Everything in this document — and in
  [`interactions.md`](./interactions.md) — describes the v0 core
  single-agent case: one `Conversation`, one `currentSessionId`, one
  pinned agent version at a time.
- **Scope, Principal, and on-behalf-of** — how `scope` and
  `initiatingPrincipal` resolve, and how an assistant acts *as* a
  member while remaining scope-pinned — is S2's normative territory.
  This document treats them only as opaque fields on `Conversation`.
