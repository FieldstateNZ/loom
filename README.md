# Loom

**The reference server implementation of [OASP](https://github.com/oasp-dev/oasp-standard), the Open Agent Session Protocol.**

OASP is built on one structural insight: **a Conversation is not a Session.**
The durable thread a user cares about should outlive the disposable provider
execution context it currently rides on. Loom is the server that proves it: a
multi-tenant agent-session gateway that holds conversations under tension while
provider sessions come and go. Hence the name. The conversation is the warp;
sessions are the weft.

Loom also refuses the industry default of normalising every provider down to an
OpenAI-shaped lowest common denominator. That approach is lossy: Anthropic's
server-side web search, code execution, MCP connector, prompt caching and
extended thinking simply disappear. Loom owns **one rich conversation
abstraction** and lets pluggable **provider libraries** translate it to each
provider's **native wire format**, preserving every managed capability verbatim
(and carrying anything it doesn't yet model through a `ProviderExtension`
escape hatch, so no feature is ever dropped).

Anthropic is the first provider, including its managed capabilities.

```
                 ┌──────────────────────────────────────────────┐
   client  ─────▶│  loom-server (axum)   /v1 conversations, SSE  │
  (virtual key)  │  auth · tenancy · budgets · usage · openapi   │
                 └───────────────┬───────────────┬──────────────┘
                                 │               │
                    ┌────────────▼───┐   ┌───────▼──────────┐
                    │  loom-core     │   │  loom-store      │
                    │  fluent        │   │  PostgreSQL      │
                    │  conversation  │   │  (sqlx)          │
                    │  domain model  │   └──────────────────┘
                    └────────┬───────┘
                             │  Provider trait + capabilities
                    ┌────────▼────────────┐
                    │  loom-provider      │  registry · negotiation · TurnEvent
                    └────────┬────────────┘
                             │  native translation (lossless)
                    ┌────────▼────────────────────┐
                    │  loom-provider-anthropic     │──▶ Anthropic Messages API
                    │  messages · streaming · MCP  │    (native wire format)
                    │  caching · server tools      │
                    └──────────────────────────────┘
```

## Loom and OASP

[OASP](https://oasp.dev) is a vendor-neutral standard for agent conversations
that outlive their execution context, with first-class identity and audit. The
standard is the product; Loom is its reference server. In practice:

- **Resource model** — OASP defines AgentDefinition, Deployment, Conversation,
  Session, Event, Principal, AuditEvent and Credential (plus
  AgentDefinitionVersion, the definition's versioning companion). Loom's
  conversation domain is converging on that model, resource by resource.
- **Conformance** — the standard ships an executable conformance kit. Loom
  tracks the v1alpha1 draft, and conformance is the acceptance bar for the work
  landing here.
- **Adapters** — OASP's adapter contract plays the role Loom's provider trait
  plays today. Anthropic is the reference adapter in both.

Spec prose, Zod-first schemas, generated JSON Schema/OpenAPI, and the v0
concept draft live in
[oasp-dev/oasp-standard](https://github.com/oasp-dev/oasp-standard) (Apache-2.0).

## Workspace crates

| Crate | Responsibility |
| --- | --- |
| `loom-core` | The fluent conversation domain model (conversations, messages, content parts, usage, options). No provider assumptions. |
| `loom-provider` | Provider plugin trait, capability model, capability negotiation, `TurnEvent` streaming envelope, provider registry. |
| `loom-provider-anthropic` | Anthropic provider: Messages API translation (non-streaming + SSE), prompt caching, server-side tools, MCP connector, batches. |
| `loom-store` | PostgreSQL persistence via `sqlx` — tenants, virtual keys, credentials, conversations, messages, usage events. |
| `loom-server` | The HTTP gateway (`axum`): `/v1` conversation endpoints, auth middleware, budgets, usage rollups, OpenAPI. |

## Web console

[`console/`](./console) is the operator-facing web console — a Vite + React 18 +
TypeScript SPA for spend, keys, budgets, MCP servers, conversations, tenants and
provider credentials, role-scoped to a gateway operator or a single tenant admin.
It codes against a small `LoomClient` interface and ships a mock-data client for
design/dev; pointing it at a running gateway is a one-file drop-in
(`createHttpClient` over the `/admin`, `/v1` and `/openapi.json` endpoints). See
[`console/README.md`](./console/README.md).

```bash
cd console && npm install && npm run dev
```

## Client SDKs

[`clients/typescript/`](./clients/typescript) is the TypeScript client for Loom —
`@fieldstate/loom-client`, a workspace artefact (not yet published to npm). It
pairs types generated from the gateway's `/openapi.json` (`src/generated.ts`,
committed alongside the `openapi.json` snapshot) with a small **fluent wrapper**:

```ts
import { createLoomClient } from "@fieldstate/loom-client";

const loom = createLoomClient({ baseUrl, apiKey });
const convo = loom.conversation({ model: "claude-haiku-4-5-20251001" });
convo.withMcp("lucidbrain").cached();          // mcp_servers + auto_cache
const message = await convo.send(userTurn);     // non-streaming -> assistant Message
for await (const ev of convo.stream(userTurn)) { /* TurnEvents (parsed SSE) */ }
```

It covers create/fetch conversation, non-streaming `send`, streaming `stream`
(an async iterator over `TurnEvent`s), `withMcp`/`cached`/server tools, and a
stateless-turn helper. See [`clients/typescript/README.md`](./clients/typescript/README.md)
and the first-consumer spike in
[`docs/spikes/lucidbrain-integration.md`](./docs/spikes/lucidbrain-integration.md).

```bash
cd clients/typescript && npm install && npm run build && npm test
```

## Status

> ⚠️ **Early development.** The scaffold, domain model, provider abstraction and
> Anthropic provider are landing issue-by-issue. The current focus is OASP
> conformance: aligning Loom's domain and API with the
> [OASP v1alpha1 draft](https://github.com/oasp-dev/oasp-standard) as the
> standard stabilises. Not production-ready yet.

![CI](https://github.com/fieldstatenz/loom/actions/workflows/ci.yml/badge.svg)

## Dev quickstart

```bash
# Build & test the whole workspace
cargo build --workspace
cargo test  --workspace

# Lint the way CI does
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings

# Run the gateway + PostgreSQL locally
docker compose up --build
curl -s localhost:8080/healthz      # -> ok
```

## Configuration

The server reads configuration from the environment (see `docker-compose.yml`
for the canonical set) and validates every secret eagerly at boot, failing fast
with a clear diagnostic if one is missing or malformed.

| Variable | Required | Default | Purpose |
| --- | --- | --- | --- |
| `DATABASE_URL` | yes | — | PostgreSQL connection URL. |
| `LOOM_ROOT_ADMIN_TOKEN` | yes | — | Bearer token guarding the `/admin` API. Compared in constant time. |
| `LOOM_ENCRYPTION_KEY` | yes | — | 32-byte AES-256-GCM key, hex-encoded (64 hex chars). Encrypts provider credentials at rest. |
| `LOOM_BIND_ADDR` | no | `0.0.0.0:8080` | `host:port` to bind the HTTP listener. |
| `LOOM_KEY_PEPPER` | no | derived | Pepper for the virtual-key lookup HMAC. When unset it is derived deterministically as `HMAC-SHA256(LOOM_ENCRYPTION_KEY, "loom.virtual-key.pepper.v1")`. Set it explicitly to decouple its lifecycle from the encryption key. |
| `LOOM_RUN_MIGRATIONS` | no | `true` | Apply database migrations on startup; set `false`/`0`/`no`/`off` to skip. |
| `LOOM_BATCH_POLL_INTERVAL_SECS` | no | `5` | Seconds between batch poll-worker passes (advancing async batch jobs). `0` disables the in-process worker (e.g. when a dedicated worker process owns it). |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | no | — | OTLP collector endpoint. **Its presence turns telemetry export on**; unset means JSON logs only, no exporter (the default). Signal-specific `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` / `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT` override it. |
| `OTEL_EXPORTER_OTLP_PROTOCOL` | no | `grpc` | OTLP transport: `grpc` (tonic) or `http/protobuf` (HTTP). |
| `OTEL_SERVICE_NAME` | no | `loom-server` | Logical service name on emitted spans/metrics. |
| `OTEL_RESOURCE_ATTRIBUTES` | no | — | Extra resource attributes, e.g. `deployment.environment=prod,service.namespace=loom`. |
| `LOOM_TELEMETRY_CAPTURE_CONTENT` | no | `0` | **Debug/privacy-sensitive.** When set (`1`/`true`/`yes`/`on`), attaches prompt/completion content to spans. Off by default; leave off wherever telemetry leaves a trusted boundary. |

Generate an encryption key with `openssl rand -hex 32`.

### Observability

Loom sits in the middle of every request, so it is instrumented from day one with
[OpenTelemetry](https://opentelemetry.io/) traces and metrics on top of the JSON
structured logs. Export is **opt-in by endpoint**: with no
`OTEL_EXPORTER_OTLP_ENDPOINT` set (tests, local dev) the server logs as usual and
installs no exporter, so no collector is required. Set an endpoint to stream OTLP
to any compatible backend (both `grpc` and `http/protobuf` transports are
supported, selected by `OTEL_EXPORTER_OTLP_PROTOCOL`).

**Spans** nest as HTTP request → conversation turn → provider call → store
operation. The provider span carries `gen_ai.request.model`, input/output and
cache token counts, computed cost, the stop reason and the tenant id; on a
streamed turn it stays open across the whole SSE stream and records first-token
latency as a span event. Every request reads or generates an `x-request-id`,
attaches it to the request span (and thus every log for that request), and echoes
it on the response.

**Metrics** (OTLP): request count + duration histogram by route and status,
provider-call duration histogram, input/output token counters by tenant + model,
a spend counter by tenant + model, an active-streams up/down counter, and a
budget-block counter by scope.

**Privacy.** By default **no** message text, tool inputs/outputs or system prompts
appear in any span, metric or log — only token counts, model, tenant, cost and
stop reason. Setting `LOOM_TELEMETRY_CAPTURE_CONTENT=1` opts in to attaching
content to spans for debugging; it is privacy-sensitive and must stay off wherever
telemetry leaves a trusted boundary.

**See traces locally.** The compose file bundles an optional OTLP collector +
dashboard (the .NET Aspire dashboard) behind the `observability` profile:

```bash
# Start the stack with the collector, then uncomment the OTEL_* vars on the
# loom-server service in docker-compose.yml.
docker compose --profile observability up --build
# Traces, metrics and logs at http://localhost:18888
```

Any OTLP backend works the same way — the OpenTelemetry Collector, Grafana Alloy,
Jaeger (with OTLP enabled), Honeycomb, and so on.

### Endpoints

| Method & path | Auth | Purpose |
| --- | --- | --- |
| `GET /healthz` | none | Liveness (always `200 ok`). |
| `GET /readyz` | none | Readiness — pings the database; `503` if unreachable. |
| `GET /openapi.json` | none | The generated OpenAPI 3.x document for the API. |
| `GET /v1/whoami` | virtual key | Echoes the authenticated tenant context. |
| `POST /v1/conversations` | virtual key | Create a conversation `{ provider, model, system?, metadata? }`. |
| `GET /v1/conversations/{id}` | virtual key | Fetch a conversation with a page of its history (`?limit&offset`); `404` across tenants. |
| `POST /v1/conversations/{id}/turns` | virtual key | Append a user turn, run the provider, return/stream the assistant turn `{ content, stream?, options? }`. |
| `DELETE /v1/conversations/{id}` | virtual key | Delete a conversation (tenant-scoped). |
| `POST /v1/turns` | virtual key | Stateless turn over a fully-inline conversation `{ provider, model, system?, messages, options?, stream? }` — no persistence. |
| `POST /v1/batches` | virtual key | Create an async batch from `{ items: [{ custom_id?, provider, model, system?, messages, options? }] }` — bulk stateless turns at the discounted batch tier. |
| `GET /v1/batches/{id}` | virtual key | Batch status and per-status item counts; `404` across tenants. |
| `GET /v1/batches/{id}/results` | virtual key | Per-item results as streamed JSONL (one `{ custom_id, status, result }` per line). |
| `POST /v1/batches/{id}/cancel` | virtual key | Request cancellation of a batch. |
| `POST /admin/tenants` | root token | Create a tenant `{ slug, name }`. |
| `GET /admin/tenants/{id}` | root token | Fetch a tenant. |
| `POST /admin/tenants/{id}/keys` | root token | Mint a virtual key `{ name, env? }`. |
| `DELETE /admin/keys/{id}` | root token | Revoke a virtual key (effective immediately). |
| `PUT /admin/tenants/{id}/credentials/{provider}` | root token | Store an encrypted provider credential `{ api_key, base_url? }`. |
| `PUT`/`DELETE /admin/tenants/{id}/budget` | root token | Set or clear a tenant's default spend budget `{ limit_amount, window, action }`. |
| `PUT`/`DELETE /admin/keys/{id}/budget` | root token | Set or clear a key's budget override (takes precedence over the tenant default). |
| `PUT`/`DELETE /admin/keys/{id}/rate-limit` | root token | Set or clear a key's rate limit `{ requests_per_min?, tokens_per_min? }`. |
| `GET /v1/usage` | virtual key | Tenant-scoped usage/cost rollups (`?from&to&group_by=key\|model\|conversation`). |
| `GET /admin/usage?group_by=tenant` | root token | Gateway-wide usage/cost rolled up by tenant. |

### Auth model

Tenant requests present a virtual key as `Authorization: Bearer loom_<env>_<...>`
(256 bits of CSPRNG entropy; `env` is `live` or `test`). The plaintext key is
shown **once** at creation and never stored. The gateway stores only a
deterministic peppered lookup hash — `key_hash = HMAC-SHA256(pepper, key)` (hex)
— plus a non-secret display prefix. A slow salted KDF (argon2) is unnecessary
here because the keys are already high-entropy, and it would break the O(1)
`key_hash` lookup; the server-side pepper means a database-only compromise still
cannot recover or forge keys. Revocation is uncached, so it takes effect on the
next request. All errors use the envelope
`{ "error": { "code", "message", "provider_error"? } }`; auth failures return
`401`.

### Admin bootstrap

With the server running and `LOOM_ROOT_ADMIN_TOKEN` set:

```bash
ADMIN=$LOOM_ROOT_ADMIN_TOKEN
BASE=http://localhost:8080

# 1. Create a tenant.
TENANT=$(curl -s -X POST $BASE/admin/tenants \
  -H "Authorization: Bearer $ADMIN" -H 'Content-Type: application/json' \
  -d '{"slug":"acme","name":"Acme Inc"}' | jq -r .id)

# 2. Mint a virtual key (the plaintext `key` is shown only here).
curl -s -X POST $BASE/admin/tenants/$TENANT/keys \
  -H "Authorization: Bearer $ADMIN" -H 'Content-Type: application/json' \
  -d '{"name":"primary","env":"live"}'
# -> { "id": "...", "key": "loom_live_...", "key_prefix": "loom_live_AbC123", ... }

# 3. Store the tenant's Anthropic credential (encrypted at rest).
curl -s -X PUT $BASE/admin/tenants/$TENANT/credentials/anthropic \
  -H "Authorization: Bearer $ADMIN" -H 'Content-Type: application/json' \
  -d '{"api_key":"sk-ant-..."}'

# 4. Call the gateway with the virtual key.
curl -s $BASE/v1/whoami -H "Authorization: Bearer loom_live_..."
```

### Conversations (`/v1`)

With a virtual key and the tenant's Anthropic credential in place (steps above),
drive a conversation. Each turn persists the user message, runs the bound
provider, and persists the assistant reply.

```bash
KEY=loom_live_...            # the virtual key from step 2
BASE=http://localhost:8080

# 1. Create a conversation bound to a provider + model.
CONVO=$(curl -s -X POST $BASE/v1/conversations \
  -H "Authorization: Bearer $KEY" -H 'Content-Type: application/json' \
  -d '{"provider":"anthropic","model":"claude-opus-4-8","system":"You are concise."}' \
  | jq -r .id)

# 2. Send a turn; the assistant message comes back as JSON.
curl -s -X POST $BASE/v1/conversations/$CONVO/turns \
  -H "Authorization: Bearer $KEY" -H 'Content-Type: application/json' \
  -d '{"content":[{"type":"text","text":"Hello, Loom!"}]}'

# 3. Stream a turn as Server-Sent Events (each `data:` frame is a TurnEvent,
#    carrying both the normalised envelope and the verbatim native event).
curl -sN -X POST $BASE/v1/conversations/$CONVO/turns \
  -H "Authorization: Bearer $KEY" -H 'Content-Type: application/json' \
  -d '{"content":[{"type":"text","text":"Stream this."}],"stream":true}'

# 4. Read the history back (paginated).
curl -s "$BASE/v1/conversations/$CONVO?limit=50" \
  -H "Authorization: Bearer $KEY"

# 5. A stateless turn — the whole conversation inline, nothing persisted.
curl -s -X POST $BASE/v1/turns \
  -H "Authorization: Bearer $KEY" -H 'Content-Type: application/json' \
  -d '{"provider":"anthropic","model":"claude-opus-4-8",
       "messages":[{"role":"user","content":[{"type":"text","text":"One-shot."}]}]}'
```

Capability negotiation runs before any request is dispatched: asking a model for
a feature it does not support returns `422` with a `capability_unsupported`
detail rather than silently degrading. A provider's own HTTP errors are mapped
through with the native payload preserved under `error.provider_error`.

### Budgets and rate limits

Every turn is checked **before** the provider call against the caller's budget
and rate limit.

**Budgets** attach at the tenant level (a default) and/or the key level; a
**key-level budget overrides the tenant default**. A budget is
`{ limit_amount, window, action }` where `window` is `daily`, `weekly`,
`monthly` (rolling look-back windows) or `total` (all time), and `action` is
`block` or `warn`. Current-window spend is summed from the priced
`usage_events` (the spend-tracking store), memoised in a short-TTL in-process
cache so a burst of turns shares one query. A key-scoped budget meters that
key's spend; a tenant-scoped budget meters the whole tenant's spend.

- `action: block` — once spend reaches the limit, the turn is rejected with
  `402` and the envelope
  `{ "error": { "code": "budget_exceeded", "details": { scope, limit, spent, window } } }`.
- `action: warn` — the turn proceeds but the response carries an
  `x-loom-budget-warning` header (and a warning is logged).

```bash
# Block a key at $25 of monthly spend.
curl -X PUT $BASE/admin/keys/$KEY_ID/budget \
  -H "Authorization: Bearer $ADMIN" -H 'Content-Type: application/json' \
  -d '{"limit_amount":"25.00","window":"monthly","action":"block"}'
```

**Rate limits** attach per key as `{ requests_per_min?, tokens_per_min? }`
(either dimension may be omitted for unlimited). They are enforced by an
in-process token bucket; a request over the limit is rejected with `429` and a
`Retry-After` header (whole seconds). Token usage is debited from the
tokens-per-minute bucket once a turn's usage is known.

```bash
# Cap a key at 60 requests and 120k tokens per minute.
curl -X PUT $BASE/admin/keys/$KEY_ID/rate-limit \
  -H "Authorization: Bearer $ADMIN" -H 'Content-Type: application/json' \
  -d '{"requests_per_min":60,"tokens_per_min":120000}'
```

> **Known limitation — single-instance enforcement.** The rate-limit token
> buckets and the budget spend cache live in each replica's memory, so both are
> enforced **per replica**: with _N_ gateway replicas the effective rate ceiling
> is _N×_ the configured limit, and a replica may not see another replica's
> just-recorded spend until its cache TTL lapses. **Distributed rate limiting and
> a shared spend cache (e.g. Redis) are deferred** to a later issue; for a
> single-instance deployment the limits are exact.

### Batches

`POST /v1/batches` accepts a list of items, each the same inline shape as
`POST /v1/turns` (`{ custom_id?, provider, model, system?, messages, options? }`),
and processes them asynchronously at the provider's **discounted batch tier**.
The lifecycle is `created → in_progress → ended` (a cancel passes through
`canceling`), advanced by an in-process poll worker; there is **no external
queue**. A worker pass is a single function (`run_batch_poll_pass`) that submits
`created` jobs, polls the provider, and finalises ended ones — so it is driven
directly (and deterministically) by tests, and on a fixed
`LOOM_BATCH_POLL_INTERVAL_SECS` interval in production.

```bash
# Submit a two-item batch.
BATCH=$(curl -s -X POST $BASE/v1/batches -H "Authorization: Bearer $KEY" \
  -H 'Content-Type: application/json' -d '{"items":[
    {"custom_id":"a","provider":"anthropic","model":"claude-opus-4-8",
     "messages":[{"role":"user","content":[{"type":"text","text":"one"}]}]},
    {"custom_id":"b","provider":"anthropic","model":"claude-opus-4-8",
     "messages":[{"role":"user","content":[{"type":"text","text":"two"}]}]}
  ]}' | jq -r .id)

curl -s $BASE/v1/batches/$BATCH -H "Authorization: Bearer $KEY"          # status + counts
curl -s $BASE/v1/batches/$BATCH/results -H "Authorization: Bearer $KEY"  # JSONL, one line/item
curl -s -X POST $BASE/v1/batches/$BATCH/cancel -H "Authorization: Bearer $KEY"
```

Two design decisions worth calling out:

- **Results retention — stored, not fetched-through.** When a batch ends the
  worker retrieves the provider's JSONL results **once** and persists each item's
  outcome into `batch_items.result`; `GET .../results` then streams straight from
  the store. Reads are therefore cheap and independent of the provider's
  results-URL retention window, at the cost of storing the result payloads
  (bounded by the batch the caller submitted).
- **Batch pricing — a multiplier, not duplicate price rows.** Rather than
  maintain a parallel set of batch-tier prices, `model_prices` carries a single
  `batch_multiplier` column (`1.0` = no discount, seeded `0.5` for Anthropic's
  50%-off batch tier). It scales only the **token** charges when a batch item's
  usage is recorded; server-tool per-request charges are unaffected. Those usage
  events are marked `is_batch = true` so rollups can distinguish batch from
  interactive spend.

## Licence

[Apache-2.0](./LICENSE) — a permissive licence: use, modify, and redistribute Loom,
including as a hosted network service, provided you retain the copyright and licence
notices (see [`NOTICE`](./NOTICE)).

Internal crate names are `loom-*`; a future crates.io publish would be prefixed
`fieldstate-loom-*` (the `loom` name is taken). Not published yet.
