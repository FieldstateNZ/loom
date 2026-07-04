# Spike: LucidBrain integration + TypeScript client (issue #17)

**Status:** spike complete — working TS client artefact + measured local run.
**Author tooling:** `clients/typescript` (the `@fieldstate/loom-client` workspace
artefact), `clients/typescript/mock/anthropic.mjs` (mock Anthropic backend),
`clients/typescript/scripts/spike.mts` (the runnable flow below).

This spike answers one question: **is Loom ready to be a first consumer's
gateway?** Concretely, can a TypeScript app drive a LucidBrain-shaped
memory-recall flow — multi-turn conversation, an MCP connector, prompt caching,
streaming, and usage/spend rollups — through Loom with an ergonomic client? The
outcome is a working client + this findings report, **not** a production
migration.

## TL;DR

- A fluent TypeScript client (`createLoomClient(...).conversation(...).withMcp('lucidbrain').cached()`)
  was built over types generated from Loom's live `/openapi.json`. Typecheck,
  build, and a request-shaping unit test all pass.
- The representative flow ran **end to end** against a local Postgres +
  `loom-server` + a mock Anthropic backend: multi-turn conversation, a named
  `lucidbrain` MCP reference reaching the provider, prompt-cache write→read
  lifecycle, a streamed turn yielding normalised `TurnEvent`s, and a
  tenant-scoped usage rollup with the priced cache read/write split.
- Loom's per-turn latency overhead vs. calling the backend directly is
  **~16 ms mean / ~17 ms p95** in this local setup — dominated by the
  synchronous Postgres round-trips a *stateful* turn performs (auth, load
  conversation, append user + assistant message, record usage).
- The single biggest ergonomics gap: **Loom's rich domain enums are opaque
  (`Object`) in the OpenAPI document**, so `openapi-typescript` types the
  request bodies but not the responses (`Message`, `TurnEvent`, `Conversation`,
  usage rollup). We hand-authored `src/types.ts` mirroring the Rust serde
  shapes. This works but risks drift. See follow-ups.

## Honesty about the environment

"Staging" here is **not** a hosted staging environment against live Anthropic.
It is a local, docker-compose-style run:

- **Postgres 16** in Docker (`loomspikepg`).
- **`loom-server`** (the release debug binary) with migrations, bound to
  `127.0.0.1:8080`.
- A **mock Anthropic backend** (`mock/anthropic.mjs`) implementing
  `POST /v1/messages` for both non-streaming and streaming (`stream:true`),
  because there is no live Anthropic key in this environment and the release
  binary ships no mock provider. The tenant's `anthropic` credential is stored
  with `base_url` pointing at the mock, so the real `AnthropicProvider` code
  path (request translation, SSE parsing, usage extraction, cache negotiation)
  is exercised — only the model itself is mocked.

**What this does NOT validate:** real Anthropic prompt-cache accounting, real
MCP-connector brokering by Anthropic, real token counts/pricing, and real
network latency. The mock returns *canned* usage (including
`cache_creation_input_tokens` / `cache_read_input_tokens`) so the plumbing —
Loom's parsing, attribution, pricing math, and rollups — is observable, but the
numbers are synthetic. A full live validation needs real Anthropic credentials
and is called out as a follow-up.

## What was exercised, and how

The flow (`scripts/spike.mts`, run with `npm run spike`) does, all through the
fluent client with a freshly-minted virtual key:

| Capability | How it was exercised | Observed |
| --- | --- | --- |
| **Provisioning** | `/admin` (root token): create tenant, mint key, PUT `anthropic` credential (base_url → mock), register `lucidbrain` MCP server | key resolves via `GET /v1/whoami` |
| **Multi-turn** | `convo.send(...)` twice on a persisted conversation | 6 persisted messages (3 user + 3 assistant incl. the streamed turn) |
| **MCP connector** | `.withMcp('lucidbrain')` on every turn | mock recorded `mcp_servers:[{name:'lucidbrain', url, authorization_token}]` — the gateway injected the URL + decrypted token **server-side** (the client never sent them) |
| **Prompt caching** | `.cached()` (auto_cache) | turn 1 `cache_write_tokens=128`; turn 2 `cache_read_tokens=128` — the write→read lifecycle |
| **Streaming** | `for await (const ev of convo.stream(...))` | 9 `TurnEvent`s: `turn_started`, `content_part_started`, `content_part_delta*`, `content_part_complete`, `turn_ended` (with final usage), plus `other` (message_stop); text reassembled from deltas |
| **Usage rollup** | `loom.usage({ group_by: 'model' })` | one row for the model: `event_count=3`, `cache_write_tokens=128`, `cache_read_tokens=256`, priced `cost=0.0006116`, split into `interactive_cost` vs `batch_cost` |

All assertions in the spike pass, and the run is reproducible (verified across
repeated runs — the mock's cache lifecycle is stateless/deterministic).

## Measured latency overhead

Method: after warming both paths, run N=60 single logical requests through Loom
(a stateful conversation turn) vs N=60 requests **directly** to the mock
Anthropic backend, timing each. Both Loom and the mock are on loopback; the mock
does no DB I/O. Representative run (numbers vary run-to-run by ~1 ms):

| Path | mean | p50 | p95 |
| --- | ---: | ---: | ---: |
| Direct → mock Anthropic | 0.75 ms | 0.70 ms | 0.92 ms |
| Through Loom (stateful turn) | 16.73 ms | 16.52 ms | 18.01 ms |
| **Loom overhead** | **15.98 ms** | **15.82 ms** | **17.09 ms** |

**Interpretation.** The ~16 ms is *not* framework overhead in the request hot
path — it is dominated by the **synchronous Postgres round-trips a stateful turn
makes**: resolve the virtual key, load the conversation + its history, append
the user message, append the assistant message, and record the priced usage
event. Against a real Anthropic backend (hundreds of ms to seconds per turn,
plus real network), this fixed ~16 ms is in the low single-digit-percent range
and would be dwarfed by model latency. Two caveats worth measuring separately:
(1) the **stateless** `/v1/turns` path skips the persistence writes and should
show materially lower overhead; (2) DB latency here is a local container — a
networked managed Postgres would raise the floor. See follow-ups.

## Abstraction gaps & ergonomics findings

Concrete things found while building the client:

1. **Domain enums are opaque in OpenAPI.** utoipa annotates the request bodies
   (`TurnRequest`, `StatelessTurnRequest`, `CreateConversationRequest`, batch
   types) but the rich, internally-tagged `#[non_exhaustive]` domain types
   (`ContentPart`, `Message`, `TurnEvent`, `Usage`, `ConversationOptions`,
   `Conversation`) are declared `#[schema(value_type = Object)]` or aren't
   `ToSchema` at all. Response envelopes (`WhoAmI`, the usage rollup) are plain
   `Serialize`, absent from the spec entirely. **Consequence:**
   `openapi-typescript` produces `Record<string, never>` / `unknown` for almost
   everything a consumer reads back. We hand-wrote `src/types.ts` mirroring the
   Rust serde representation. It works and is well-tested by the live run, but
   it is a **second source of truth that can silently drift** from the Rust
   types.

2. **Named MCP references need out-of-band registration, discovered only at
   turn time.** `convo.withMcp('lucidbrain')` compiles and sends fine, but if
   the tenant hasn't registered a `lucidbrain` MCP server via
   `PUT /admin/tenants/{id}/mcp-servers/lucidbrain`, the turn fails with a
   `422 mcp_server_not_configured`. There's no client-side pre-flight or a way
   to list registered servers from the tenant-scoped API. The spike had to add
   the admin registration step. This is the right security model (tokens never
   transit the client), but the failure mode is late and undiscoverable.

3. **No single "result" object for a stream.** The streaming API is a clean
   async iterator of `TurnEvent`s, which is the right low-level primitive. But
   every consumer that just wants "the final assistant message + its usage" has
   to re-implement the fold over `content_part_complete` / `turn_ended` events
   (Loom already has this logic server-side as `TurnAccumulator`). A
   `collect(stream)` helper in the client would remove boilerplate.

4. **Per-turn cost/usage is not on the turn response.** A non-streaming
   `send()` returns the assistant `Message` (which carries provider `usage`),
   but the **priced cost** and Loom's attribution are only visible later via
   `/v1/usage`, which is eventually-consistent (usage is written through a
   best-effort recorder/outbox). A consumer wanting immediate per-turn spend has
   no direct field. For streaming, usage rides the `turn_ended` event only.

5. **Streamed assistant turns persist with `raw = null`.** Already documented in
   `v1.rs` as a known asymmetry: the non-streaming path persists the verbatim
   native response blob, the streaming path does not (though every SSE frame
   carries its native event on `TurnEvent.raw`). Not a blocker for the client,
   but a consumer relying on `Message.raw` for audit will find it absent on
   streamed turns fetched from history.

6. **Auth is a bearer virtual key; admin is a separate root token.** Clean and
   unambiguous. The client only needs the virtual key. Provisioning
   (tenants/keys/credentials/MCP servers) is deliberately admin-only and lives
   outside the consumer client — correct, but means a consumer's onboarding is a
   two-actor dance (operator provisions, app consumes).

Things that were pleasantly friction-free: the `ConversationOptions` shape maps
directly to a fluent builder; `auto_cache` + `mcp_servers` are exactly the two
knobs the LucidBrain flow needs; SSE frames are well-formed `TurnEvent` JSON
that parse without provider-specific handling; and the usage rollup already
splits interactive vs batch cost and cache read vs write, which is precisely
what a spend dashboard wants.

## Proposed follow-up issues

Ready-to-file (not filed here):

1. **Expose Loom's domain model in OpenAPI** — add `utoipa::ToSchema` (with
   proper `oneOf`/discriminator mappings) for `ContentPart`, `Message`,
   `TurnEvent`/`TurnEventKind`, `ContentDelta`, `Usage`, `ConversationOptions`,
   `McpServerRef`, `ServerTool`, and `Conversation`, so generated clients are
   fully typed on responses, not just request bodies.
2. **Add `ToSchema` to response envelopes** — `WhoAmI`, `UsageRollupResponse` /
   `UsageRollupRowDto`, and the admin usage response are currently absent from
   the spec. Type them so `/v1/whoami` and `/v1/usage` are in the generated
   surface.
3. **TS client: `collect(stream)` helper** — reassemble the final `Message` +
   `Usage` + `StopReason` from a `TurnEvent` iterator (mirroring the server-side
   `TurnAccumulator`), so streaming consumers don't re-implement the fold.
4. **Discoverable MCP references** — a tenant-scoped `GET /v1/mcp-servers`
   (names only) and/or a client pre-flight so `withMcp(name)` can fail fast with
   a helpful message instead of a late `422` at turn time.
5. **Per-turn usage + cost on the turn response** — return the priced usage /
   cost (or add a per-turn usage lookup) so consumers get spend without polling
   the eventually-consistent `/v1/usage` rollup.
6. **CI drift guard for the generated client** — in CI, start `loom-server`,
   curl `/openapi.json`, regenerate `src/generated.ts`, and fail if it differs
   from the committed snapshot (catches spec/type drift). Decide whether to
   publish `@fieldstate/loom-client` to a registry.
7. **Measure stateless vs stateful turn overhead** — quantify the `/v1/turns`
   (no-persistence) path against the stateful path, and evaluate reducing the
   per-turn Postgres round-trips (batched writes / pipelining) for the ~16 ms
   fixed cost seen here.
8. **Typed error taxonomy in the client** — map `ApiError.code` values
   (`mcp_server_not_configured`, budget/rate-limit codes, capability errors) to
   discriminated `LoomError` subtypes so consumers can branch on failure kind.
9. **Full live-Anthropic validation** — re-run this flow against real Anthropic
   credentials to validate real cache accounting, MCP brokering, and pricing
   (this spike used a mock backend by necessity).

## Reproducing the spike

```bash
# 1. Postgres
docker run -d --name loomspikepg -e POSTGRES_PASSWORD=loom -e POSTGRES_USER=loom \
  -e POSTGRES_DB=loom -p 5432:5432 docker.io/library/postgres:16

# 2. loom-server
DATABASE_URL=postgres://loom:loom@127.0.0.1:5432/loom \
LOOM_ROOT_ADMIN_TOKEN=spike-root LOOM_ENCRYPTION_KEY=$(openssl rand -hex 32) \
LOOM_BIND_ADDR=127.0.0.1:8080 LOOM_RUN_MIGRATIONS=true \
  cargo run -p loom-server

# 3. mock Anthropic backend + the flow
cd clients/typescript
npm ci
node mock/anthropic.mjs 8790 &        # mock backend
npm run spike                          # provisions + runs the flow, prints a JSON report
```

The client itself is provider- and mock-agnostic: `npm run build` /
`npm run typecheck` / `npm test` need none of the above.
