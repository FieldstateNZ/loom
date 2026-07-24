# Adapters

> Prerequisite reading: [`docs/oasp-v0-concept.md`](../oasp-v0-concept.md)
> § Adapter contract and § Event; [`interactions.md`](./interactions.md),
> whose [`stream`](./interactions.md#stream) section states the
> lexicographic event-ordering guarantee this document elevates to a
> preserve-not-lose invariant, and whose
> [Stage 2 note](./interactions.md#stage-2--transcript-seed-with-a-suppression-marker)
> explicitly defers the suppression-marker transport to this document;
> [`conversation-and-session.md`](./conversation-and-session.md), which
> specifies the `Session` shape (`pinnedAgentVersion`, `resources`,
> `vaultIds`) `createSession` must honour. This is S3
> ([issue #4](https://github.com/FieldstateNZ/oasp-standard/issues/4)),
> building on the S0 resource schemas
> ([issue #1](https://github.com/FieldstateNZ/oasp-standard/issues/1)):
> [`sessionSchema`](../../packages/schemas/src/resources/session.ts),
> [`eventSchema`](../../packages/schemas/src/resources/event.ts),
> [`deploymentSchema`](../../packages/schemas/src/resources/deployment.ts),
> [`agentVersionRefSchema`](../../packages/schemas/src/common/agent-version-ref.ts),
> and [`providerSchema`](../../packages/schemas/src/common/provider.ts).

An OASP server does not talk to Anthropic, OpenAI, or Google directly.
It talks to an **adapter** implementing one interface —
`AgentProvider` — and the adapter owns the translation into and out of
whatever that provider actually exposes. This document formalizes that
interface as the standard's adapter contract: what every operation
means, what a conformant adapter **MUST** preserve when translating a
provider's native behaviour into OASP's normalised vocabulary, and what
it **MAY** lose. The executable counterpart — a TypeScript encoding of
this same interface, a deterministic mock implementing it, and the
conformance checks that exercise it — lives in
[`packages/conformance`](../../packages/conformance) (`@oasp/conformance`);
see that package's README for the live-provider smoke ritual this
document defines in [Live-provider smoke](#live-provider-smoke-a-per-adapter-release-ritual).

## Why an adapter, not a client library per provider

Every provider that can host a long-running, tool-using agent shapes
its session/turn/tool-call primitives slightly differently, and none of
them model the Conversation/Session split
([`conversation-and-session.md`](./conversation-and-session.md)) at
all — that split is OASP's own invention, built on top of whatever a
provider natively offers. The adapter is where that gap is closed once,
centrally, rather than by every server implementation re-deriving it.
Translation into OASP's normalised vocabulary is **lossy by design** —
see [Preserve vs. may lose](#preserve-vs-may-lose-normative) — but the
loss is bounded and enumerated, not incidental.

## The `AgentProvider` interface

Ten operations, grouped by what they act on. Each subsection states its
normative behaviour. The canonical TypeScript encoding is
[`AgentProvider`](../../packages/conformance/src/adapter/agent-provider.types.ts)
in `@oasp/conformance`; this prose and that interface are kept in sync
deliberately — where they could be read to disagree, the TSDoc on the
interface (generated from, and reviewed alongside, this document) is
the executable expression of the same contract.

### `ensureEnvironment`

`ensureEnvironment(environmentId): Result<EnsureEnvironmentResult, AdapterError>`

- An adapter **MUST** idempotently ensure the named provider-side
  environment exists, creating it if absent and returning successfully
  if it is already present. A server **MUST** be able to call this
  repeatedly for the same `environmentId` without side effects beyond
  the first successful call.
- What an "environment" concretely is (a workspace, a project, a
  tenant-scoped execution boundary) is provider-specific; the contract
  only requires that the same `environmentId` always resolves to the
  same provider-side environment for the lifetime of that id.

### `createAgent` / `updateAgent` / `getAgent`

`createAgent(definition, environmentId): Result<Deployment, AdapterError>`
`updateAgent(providerAgentId, definition, environmentId): Result<Deployment, AdapterError>`
`getAgent(providerAgentId): Result<Deployment, AdapterError>`

- `createAgent` **MUST** materialize the given `AgentDefinition` at the
  provider within the given environment and return a
  [`Deployment`](../../packages/schemas/src/resources/deployment.ts) —
  reusing the S0 resource shape rather than a bespoke adapter-local
  type, since a materialized agent at a provider is exactly what
  `Deployment` already models.
- `updateAgent` **MUST** update the provider-side agent identified by
  `providerAgentId` to match the given `AgentDefinition`, returning the
  updated `Deployment`. An adapter **MUST NOT** create a second,
  parallel provider-side agent as a side effect of `updateAgent` — it
  updates the one named, in place, at the provider.
- `getAgent` **MUST** return the current `Deployment` for the given
  `providerAgentId` without mutating provider state.
- None of the three **MUST** implement `Deployment`'s
  `canonicalHash`-based idempotency short-circuit themselves — that
  check is the server's job, before it decides whether to call
  `createAgent`/`updateAgent` at all (see
  [`deploymentSchema`](../../packages/schemas/src/resources/deployment.ts)).
  The adapter's obligation is narrower: do what it's told, faithfully,
  against the named provider-side agent.

### `createSession`

`createSession(options: CreateSessionOptions): Result<Session, AdapterError>`

This is the operation the Conversation/Session split's every
distinctive guarantee ultimately rests on.

- An adapter **MUST** create a new provider-side execution context
  pinned to exactly the `pinnedAgentVersion` given in `options` — not a
  "latest" or "current" resolution performed adapter-side. Version
  resolution ([`target-version-resolution.md`](./target-version-resolution.md))
  is the server's job; by the time `createSession` is called, the
  target version is already decided, and the adapter's only obligation
  is to pin to exactly that value.
- An adapter **MUST** mount exactly the `resources` given — every
  entry, not a subset — and **MUST** attach exactly the `vaultIds`
  given. It **MUST NOT** silently drop, substitute, or partially apply
  either array. This is what makes `migrate`'s Stage 1 re-attachment
  requirement ([interactions.md § Stage 1](./interactions.md#stage-1--mint-session-at-target-version))
  actually hold at the adapter boundary, not just at the server's call
  site.
- The returned [`Session`](../../packages/schemas/src/resources/session.ts)
  **MUST** carry the provider-assigned `id` and echo back
  `pinnedAgentVersion`, `resources`, and `vaultIds` exactly as
  requested — a caller **MUST** be able to trust the returned `Session`
  as ground truth for what was actually created, without a separate
  confirmation round-trip.

#### `seed` — the transcript-seeding transport (normative resolution of a deferred S1 detail)

> **Note (this is an S3 resolution, flagged for the dev lead's sign-off):**
> [`interactions.md` § Stage 2](./interactions.md#stage-2--transcript-seed-with-a-suppression-marker)
> states the *semantic* requirement — seeded content must be treated as
> already exchanged, and a freshly migrated Session must not emit a
> fresh `assistant_message_start` in response to it alone — but
> explicitly defers "the exact transport of that marker" to this
> document. This subsection is that resolution.

- `CreateSessionOptions` **MAY** carry an optional `seed: { events }`
  field: an ordered array of [`Event`](../../packages/schemas/src/resources/event.ts)s
  representing the transcript to seed, already flattened and
  non-compounded by the server per
  [interactions.md's non-compounding rule](./interactions.md#non-compounding-transcript-seeding-normative).
- When `seed` is present, an adapter **MUST** cause the provider to have
  this content in context from the moment the session becomes usable,
  and **MUST NOT** produce a fresh assistant turn as a direct,
  unsolicited response to the seed alone. *How* the adapter achieves
  this — a provider-native "prior turns" parameter, a system-prompt
  injection, a synthetic non-triggering message — is adapter-specific
  and unconstrained by this document; only the observable guarantee
  (no unsolicited `assistant_message_start` in response to seeding) is
  normative.
- An adapter **MUST NOT** re-introduce compounding at its own layer:
  it receives a flat, already-de-duplicated event array from the
  server and **MUST** treat it as a single, flat batch — an adapter is
  never asked to (and must not) merge it with any transcript state it
  independently retains for the outgoing session.
- Deciding *what* to put in `seed` — fetching the outgoing session's
  transcript, degrading to an empty seed on fetch failure, stripping
  prior seed structure so the batch stays flat — is entirely the
  server's responsibility per [interactions.md](./interactions.md#stage-2--transcript-seed-with-a-suppression-marker).
  The adapter's obligation begins once `seed` is handed to it.

### `sendMessage`

`sendMessage(sessionId, content, principal?): Result<void, AdapterError>`

- An adapter **MUST** post the given content into the named session as
  a new turn, attributed to `principal` where the provider supports
  per-turn attribution (optional — not every provider surfaces this).
- This operation is the adapter-level primitive
  [interactions.md § `send`](./interactions.md#send) is built on; the
  server-level `send` interaction's current-session enforcement (only
  the Conversation's `currentSessionId` may receive new `send` traffic)
  happens above this call, not inside it — `sendMessage` itself has no
  concept of a Conversation.

### `sendToolResult`

`sendToolResult(sessionId, toolUseId, result): Result<void, AdapterError>`

- An adapter **MUST** correlate the posted `result` to the pending tool
  use identified by `toolUseId` and **MUST** reject the call (return an
  `AdapterError`, never silently no-op) if no tool use with that id is
  currently pending on the named session. This is the same correlation
  requirement [interactions.md § `sendToolResult`](./interactions.md#sendtoolresult)
  states at the server level; the adapter is where it is actually
  enforced against provider state.

### `getSessionStatus`

`getSessionStatus(sessionId): Result<SessionStatus, AdapterError>`

- An adapter **MUST** report the session's current status as one of
  `running`, `idle`, or `error` — the same three values the `status`
  [`Event`](../../packages/schemas/src/resources/event.ts) carries
  (`SessionStatus` is derived from that union, not redefined — see
  [`session-status.types.ts`](../../packages/conformance/src/adapter/session-status.types.ts)).
  `running` **MUST** be reported whenever the session is actively
  producing output or is parked waiting on a pending tool result;
  `idle` **MUST** be reported only once the session has fully settled
  with no pending tool calls outstanding.

### `listSessionEvents`

`listSessionEvents(sessionId, options?): Result<ListSessionEventsResult, AdapterError>`

- An adapter **MUST** return the session's normalised event history in
  emission order, paginated via `options.afterId`/`options.limit`,
  reconstructible into the same ordered sequence regardless of how it
  is paginated. This is both the derive-on-read fallback for clients
  that never streamed live and — per
  [interactions.md § `stream`](./interactions.md#stream) — the audit
  source; a server's ability to answer
  [the audit conformance test](./audit.md#the-conformance-test-normative)
  depends on this operation being complete and correctly ordered.
- Event ordering here **MUST** agree exactly with `streamEvents`'s
  emission order for the same session — the two are two views onto one
  ordered history, not independent sources that might disagree.

### `streamEvents`

`streamEvents(sessionId): AsyncIterable<Event>`

- An adapter **MUST** yield [`Event`](../../packages/schemas/src/resources/event.ts)s
  in true emission order (see
  [Preserve vs. may lose](#preserve-vs-may-lose-normative) below for
  the ordering guarantee itself) and **MUST** terminate the iterable
  once a `status` Event with `status: 'idle'` or an `error` Event with
  `recoverable: false` has been yielded — mirroring
  [interactions.md § `stream`](./interactions.md#stream)'s termination
  rule at the transport the server consumes directly.
- An adapter **MUST NOT** terminate the iterable merely because the
  provider's underlying transport paused (e.g. between tool-execution
  round-trips) while status remains `running`.

### `getPendingToolCalls`

`getPendingToolCalls(sessionId): Result<readonly PendingToolCall[], AdapterError>`

- An adapter **MUST** enumerate every blocking tool use the session is
  currently parked on — not a subset, not just the most recent one —
  each with its `toolUseId`, `name`, and `input` intact, and — when the
  adapter can identify it — the `mcpServerUrl` of the MCP server the
  call was routed through. This is the operation
  [`drain`](./interactions.md#drain) depends on entirely: a server
  cannot recover a parked session it cannot fully enumerate, and (per
  `drain`'s authorization clause) cannot authorize an MCP-routed call it
  cannot attribute to a granted server.
- If the session has no pending tool calls (already `idle` or still
  actively producing output with nothing blocking), the adapter
  **MUST** return an empty array rather than an error.

## Preserve vs. may lose (normative)

Translation into OASP's normalised vocabulary is lossy by design — an
adapter is not expected to be a lossless mirror of everything a
provider's native API exposes. What follows is the explicit boundary:
the invariants a conformant adapter **MUST NOT** lose, and the
categories of provider-native detail it **MAY** lose in the course of
translation.

### MUST preserve

1. **Version pinning.** `createSession`'s `pinnedAgentVersion` **MUST**
   be exactly what was requested, for the life of the session. There is
   no "adapter decided to use a newer version" escape hatch — pinning
   is the entire mechanism `publish` and `migrate` rely on to keep live
   conversations undisturbed
   ([interactions.md § `publish`](./interactions.md#publish)).
2. **Resource and credential fidelity at session creation.** Every
   entry in `resources` and `vaultIds` passed to `createSession`
   **MUST** be honoured, not partially applied — see
   [`createSession`](#createsession) above.
3. **Pending-tool-call enumeration.** `getPendingToolCalls` **MUST**
   return the complete, accurate set of blocking tool uses — `drain`'s
   correctness depends on this being total, not best-effort. When an
   adapter reports a call's `mcpServerUrl`, it **MUST** be the call's
   true origin — `drain`'s authorization clause
   ([interactions.md § `drain`](./interactions.md#drain)) trusts it to
   match the call against the pinned `AgentDefinition`'s granted `mcp`
   tools.
4. **Event ordering.** Per
   [interactions.md § `stream`](./interactions.md#stream), each
   `Event`'s `id` **MUST** be assigned so that it is monotonically
   **lexicographically** increasing in emission order within a session
   — sorting the ids as byte strings reproduces true emission order. A
   bare, non-zero-padded ascending integer does **not** satisfy this
   (`"10"` sorts before `"2"`). `listSessionEvents` pagination and the
   audit trail's ability to reconstruct "what happened, in order" both
   depend on this holding without exception.
5. **The normalised Event vocabulary.** Every unit of provider-native
   streaming output **MUST** be translated into one of the eight
   [`Event`](../../packages/schemas/src/resources/event.ts) variants
   (`assistant_message_start`/`_text`/`_end`, `assistant_thinking`,
   `custom_tool_use`, `builtin_tool_use`, `status`, `error`). An adapter
   **MUST NOT** emit anything outside this vocabulary as part of the
   normalised stream — the discriminated union is closed by the S0
   schema, so this is enforced structurally, not just by convention.
6. **`sendToolResult` correlation.** A posted result **MUST** be
   applied to the exact pending tool use its `toolUseId` names, never
   to a different pending call as a matter of adapter convenience.
7. **Stream termination semantics.** A `status: 'idle'` or
   `recoverable: false` `error` Event **MUST** actually reflect the
   session having settled or terminally failed — an adapter **MUST
   NOT** emit `idle` while a tool call is genuinely still pending, since
   that would falsely tell `drain` (and any waiting `stream` consumer)
   that there is nothing left to do.
8. **No fresh assistant turn from seeding alone.** Per
   [`seed`](#seed--the-transcript-seeding-transport-normative-resolution-of-a-deferred-s1-detail)
   above: a freshly seeded session **MUST NOT** produce an unsolicited
   `assistant_message_start` as a direct response to the seed.

### MAY lose

1. **Provider-native event/message shapes and internal identifiers.**
   Whatever ids, envelope structure, or internal metadata the
   provider's own streaming protocol uses are not required to survive
   translation — only their *content*, mapped into the normalised
   vocabulary, is preserved.
2. **Provider-specific formatting or internal reasoning detail beyond
   what is surfaced as `assistant_thinking`.** A provider that exposes
   richer internal-reasoning structure than a flat text delta **MAY**
   have that structure flattened.
3. **Provider-side session/agent identifiers beyond what addresses the
   session.** An adapter **MAY** keep its own internal id mapping
   opaque to the server, as long as the `Session.id` it returns from
   `createSession` remains stable and addressable for every other
   operation.
4. **Exact chunk-level timing or batching of provider-native streaming.**
   An adapter **MAY** coalesce multiple provider-native chunks into one
   `assistant_message_text` Event (or split one provider chunk into
   several) as long as relative order and lexicographic `id` monotonicity
   are preserved. Real-time latency characteristics are not part of the
   contract.
5. **Provider-specific tool-call metadata beyond `toolUseId`/`name`/`input`/`mcpServerUrl`.**
   Anything the provider attaches to a tool call beyond those four
   fields (invocation confidence scores, provider-internal routing
   hints, etc.) is not required to surface through `PendingToolCall` or
   the tool-use Events. `mcpServerUrl` itself **MAY** still be absent —
   a `builtin_toolset`/`custom` call has none by definition, and an
   adapter whose provider integration genuinely cannot attribute an
   MCP-routed call to a specific server is not required to fabricate
   one. Be aware that omission is **not** fail-closed under
   [interactions.md § `drain`](./interactions.md#drain)'s authorization
   clause: an unattributed call is checked against `custom` grants and
   the builtin-toolset carve-out instead, so whenever the pinned
   `AgentDefinition` grants any `builtin_toolset`, an MCP-routed call
   whose provenance the adapter dropped is indistinguishable from a
   builtin call and will be authorized. An adapter **SHOULD** therefore
   surface `mcpServerUrl` whenever its provider integration can
   attribute the call, and **MUST NOT** report an `mcpServerUrl` it
   cannot vouch for (e.g. a granted server's URL stamped onto a call
   that was not actually routed through it, waving the call past the
   allowlist/origin check).
6. **The suppression-marker transport itself** (as opposed to the
   guarantee it produces) — see [`seed`](#seed--the-transcript-seeding-transport-normative-resolution-of-a-deferred-s1-detail).
   *How* an adapter tells its provider "treat this as already
   exchanged" is unconstrained; only the resulting behaviour is
   normative.

## Reference adapter: Anthropic (Managed Agents)

Per [`providerSchema`](../../packages/schemas/src/common/provider.ts)
and the concept draft's Adapter contract section, **Anthropic is the
reference adapter** — the one the standard's own provenance traces
back to (LucidBrain's managed-agents implementation), and the one a new
conformant server implementation should look to first when building its
own adapter layer.

Anthropic's Managed Agents surface already models the two primitives
`AgentProvider` needs most directly: a persistent, provider-hosted agent
definition (`createAgent`/`updateAgent`/`getAgent`'s target) and a
session-scoped execution context created against it
(`createSession`'s target), with tool use — both provider-hosted
builtin tools and MCP-server-backed tools — and streaming output as
first-class primitives. This is why the concept draft can state
"Anthropic is the reference adapter" without further qualification: the
shape of what OASP asks an adapter to do and the shape of what
Anthropic's Managed Agents already expose are close enough that the
translation is direct, not a fight against an impedance mismatch.

Concretely, an Anthropic adapter's translation responsibilities map as
follows:

| `AgentProvider` operation | Anthropic-side concept |
|---|---|
| `createAgent` / `updateAgent` / `getAgent` | Create/update/fetch the Managed Agent definition materialized from `AgentDefinition` (model, instructions, tool grants). |
| `createSession` | Create a session-scoped execution context against that Managed Agent, pinned to its version, with `resources`/`vaultIds` mounted at creation and (when present) `seed` applied as prior context. |
| `sendMessage` / `sendToolResult` | Post a new turn / post a tool result into the running session. |
| `getSessionStatus` / `getPendingToolCalls` | Query the session's current execution state and any tool uses it is blocked on. |
| `streamEvents` / `listSessionEvents` | Consume Anthropic's native streaming output, translated into the eight-variant `Event` vocabulary, in emission order. |

Building the concrete Anthropic adapter implementation (as opposed to
documenting it as the reference) is **out of this slice's scope** — S3
formalizes the contract and ships a deterministic mock plus the
conformance kit that any real adapter, including a future Anthropic
one, must pass. `packages/conformance`'s mock provider
([`createMockAgentProvider`](../../packages/conformance/src/mock/create-mock-agent-provider.ts))
implements this same `AgentProvider` interface deterministically and
in-memory, standing in for Anthropic (or any provider) in CI.

## Reserved adapter slots: OpenAI (Responses) and Google

Per [`providerSchema`](../../packages/schemas/src/common/provider.ts),
`openai` and `google` are reserved provider identifiers — a conformant
server **MAY** accept them in data (an `AgentDefinition.provider` or
`Deployment.provider` value of `openai` or `google` is not itself a
schema violation) even though **no adapter for either ships in v0**.
`openai` names OpenAI's Responses API as the intended target; `google`
is reserved without a more specific API surface named yet. Reserving
the names now, rather than only adding them once an adapter exists,
lets `AgentDefinition`s and `Deployment`s reference a not-yet-supported
provider without a schema change later — the schema change already
happened (S0); only the adapter implementation is pending.

A server **MUST NOT** claim Level 3 (Adapter) conformance for a
provider it has not actually implemented and passed against the
conformance kit — reserving `openai`/`google` by name in the `Provider`
enum is not itself a conformance claim about either.

## Live-provider smoke: a per-adapter release ritual

The conformance kit (`packages/conformance`) runs entirely against the
deterministic mock provider — no live keys, no network, CI-safe by
construction (see that package's README). That proves an adapter
*implementation* is internally coherent against the `AgentProvider`
contract; it cannot prove a *real* provider integration actually works
end-to-end against the live API, since the mock cannot fail in the ways
a real provider can (network flakiness, provider-side rate limits,
genuine context-overflow behaviour, auth expiry).

For that, each adapter defines a **live-smoke set**: the minimal
sequence of real-provider calls run **on release, not per-PR, and never
in CI**:

1. **Happy path** — create an agent, create a session, send a message,
   observe a complete `assistant_message_start` → `_text`* → `_end` →
   `status: idle` sequence.
2. **Stream** — the same happy path, but consumed via `streamEvents`
   rather than polled via `listSessionEvents`, confirming the live
   stream terminates correctly on `idle`.
3. **Tool use** — a message that triggers at least one tool call,
   confirming `getPendingToolCalls` enumerates it and `sendToolResult`
   / `drain` correctly returns the session to `idle`.
4. **Induced error** — a request engineered to fail at the provider
   (e.g. an invalid tool result, a malformed input) confirming the
   adapter surfaces an `error` Event with the correct `recoverable`
   value rather than hanging or throwing an unstructured exception.
5. **Context overflow** — a session driven past the provider's context
   window, confirming the adapter surfaces this as a structured,
   recoverable-or-not `error` Event rather than an opaque provider-side
   failure the server cannot interpret.

A live-smoke run is a **ritual, not a gate**: it is the correctly-scoped
successor to what an earlier, broader "run live-provider tests in CI"
proposal would have been — live keys and live network calls have no
place in a per-PR or per-CI-run gate, but a real provider integration
still needs *some* periodic confirmation that reality still matches the
mock's model of it. `packages/conformance`'s automated suite
deliberately does **not** implement or invoke this set; see
[`packages/conformance/README.md`](../../packages/conformance/README.md)
for the maintained definition and how a release process should run it
by hand or in a separate, credentialed, non-CI workflow.

## Relationship to conformance levels

Per the concept draft's
[Conformance](../oasp-v0-concept.md#conformance) section and
[`README.md`'s Conformance levels](./README.md#conformance-levels)
recap, this document is what Level 3 (Adapter) conformance means
concretely: a provider mapping that preserves every invariant in
[MUST preserve](#must-preserve) above. `packages/conformance` is where
that claim becomes checkable — see its README for how a server
self-reports the conformance level(s) it claims and how the kit
verifies rather than trusts that claim.
