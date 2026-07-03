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

The server reads configuration from the environment (see `docker-compose.yml`
for the canonical set): `LOOM_BIND_ADDR`, `DATABASE_URL`,
`LOOM_ROOT_ADMIN_TOKEN`, `LOOM_ENCRYPTION_KEY`, and the standard `OTEL_*`
variables for telemetry.

## Licence

[AGPL-3.0-only](./LICENSE). If you run a modified Loom as a network service, the
AGPL requires you to offer your users the corresponding source.

Internal crate names are `loom-*`; a future crates.io publish would be prefixed
`fieldstate-loom-*` (the `loom` name is taken). Not published yet.
