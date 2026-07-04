# Loom

**A multi-tenant LLM gateway that speaks _fluent conversation_ — and never flattens a provider's native features to do it.**

Most gateways normalise every provider down to an OpenAI-shaped lowest common
denominator. That's lossy: Anthropic's server-side web search, code execution,
MCP connector, prompt caching and extended thinking simply disappear. Loom takes
the opposite bet — it owns **one rich conversation abstraction** and lets
pluggable **provider libraries** translate it to each provider's **native wire
format**, preserving every managed capability verbatim (and carrying anything it
doesn't yet model through a `ProviderExtension` escape hatch, so no feature is
ever dropped).

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

## Workspace crates

| Crate | Responsibility |
| --- | --- |
| `loom-core` | The fluent conversation domain model (conversations, messages, content parts, usage, options). No provider assumptions. |
| `loom-provider` | Provider plugin trait, capability model, capability negotiation, `TurnEvent` streaming envelope, provider registry. |
| `loom-provider-anthropic` | Anthropic provider: Messages API translation (non-streaming + SSE), prompt caching, server-side tools, MCP connector, batches. |
| `loom-store` | PostgreSQL persistence via `sqlx` — tenants, virtual keys, credentials, conversations, messages, usage events. |
| `loom-server` | The HTTP gateway (`axum`): `/v1` conversation endpoints, auth middleware, budgets, usage rollups, OpenAPI. |

## Status

> ⚠️ **Early development.** The scaffold, domain model, provider abstraction and
> Anthropic provider are landing issue-by-issue. Not production-ready yet.

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

Generate an encryption key with `openssl rand -hex 32`.

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
| `POST /admin/tenants` | root token | Create a tenant `{ slug, name }`. |
| `GET /admin/tenants/{id}` | root token | Fetch a tenant. |
| `POST /admin/tenants/{id}/keys` | root token | Mint a virtual key `{ name, env? }`. |
| `DELETE /admin/keys/{id}` | root token | Revoke a virtual key (effective immediately). |
| `PUT /admin/tenants/{id}/credentials/{provider}` | root token | Store an encrypted provider credential `{ api_key, base_url? }`. |

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

## Licence

[AGPL-3.0-only](./LICENSE). If you run a modified Loom as a network service, the
AGPL requires you to offer your users the corresponding source.

Internal crate names are `loom-*`; a future crates.io publish would be prefixed
`fieldstate-loom-*` (the `loom` name is taken). Not published yet.
