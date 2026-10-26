# OpenAI Responses adapter

`loom-provider-openai` implements the gateway's existing `Provider` interface
using `POST /v1/responses`. It is the OpenAI path needed by Tandem's current
conversation bridge. It does not claim the newer `AgentProvider` resource
lifecycle or full OASP conformance.

## Supported contract

The conservative default catalogue contains `gpt-6-luna`, preserving Tandem's
selected model. The library can accept an operator-reviewed catalogue through
`with_models`; this does not establish account access or automatically discover
new models. The gateway uses the default catalogue.

| Capability | Behavior |
| --- | --- |
| Text and image input | Native Responses input items; unsupported content is rejected before dispatch |
| Stateless provider requests | `store: false`; no `previous_response_id`, provider conversation or background job |
| Durable conversations | Existing Loom PostgreSQL history is replayed, including native assistant message IDs/phases and encrypted reasoning items |
| Function calls | `call_id`, arguments, tool metadata and correlated results preserved; the host must validate and execute tools |
| Streaming | Native SSE mapped to `TurnEvent`, including authoritative completed parts and terminal status; premature EOF is an error |
| Usage | Cached input split from full-price input; raw usage retained, including reasoning token details |
| Refusals and unknown items | Preserved as provider extensions instead of discarded or misclassified as ordinary model text |

The adapter defaults `reasoning.effort` to `none` for `gpt-6-luna`, matching
Tandem's low-latency setting. Provider-native options may change reasoning and
other native controls, but cannot replace Loom-owned history, instructions,
model, persistence, streaming mode, token limit or tool definitions.

Generic Loom function schemas are sent unchanged with `strict: false`. This
preserves optional-field semantics rather than silently normalizing them into
required fields. The executing host must validate arguments against the declared
schema. A future strict-schema option can be added without changing this default.

## Deliberately unsupported

Provider-hosted MCP, hosted search/code tools, document inputs and batches are
not advertised by this adapter. A request requiring them fails capability
negotiation rather than silently changing behavior. Private LucidBrain MCP
execution is a gateway-side follow-up, not an accidental public-MCP fallback.

Loom's explicit per-block cache hints are advisory by default and generate a
warning; `hard_fail` rejects them. Native automatic OpenAI caching still works
and is reflected in usage. Responses does not expose Loom's `stop_sequences`
control, so a supplied stop sequence is rejected rather than dropped.

The HTTP client disables redirects and accepts HTTPS base URLs or loopback HTTP
for fixtures. It does not retry requests automatically: a transport failure can
occur after inference has begun. Dropping a stream stops local consumption; it
does not prove remote cancellation or reverse tool side effects. Incomplete
streams expose their stop reason; failed/nonterminal streams never emit a
successful `end_turn`. Non-streaming incomplete responses return a structured
provider error with the native payload retained.

## Gateway configuration

Use the existing administrative credential endpoint with provider `openai`:

```text
PUT /admin/tenants/{tenant_id}/credentials/openai
{"api_key":"<private operator-provided credential>"}
```

The existing encryption/AAD, tenant scoping and virtual-key authorization are
reused. A base-URL override is available for a trusted gateway or local tests.
Neither a root token nor a provider key belongs in the mobile app.

Then create a conversation using a tenant virtual key:

```json
{"provider":"openai","model":"gpt-6-luna","system":"You are Riley."}
```

Send through `/v1/conversations/{id}/turns` as usual, with `stream: true` for SSE.
Function calls pause with `tool_use`; this adapter does not execute them. A host
can submit correlated `ToolResult` content on a subsequent turn.

Before live deployment, configure and verify a versioned OpenAI price row in
Loom's pricing store. This change does not guess billing prices or provision
credentials. Without a configured price, existing Loom behavior retains usage
with an unknown cost; cost-based budgets cannot be treated as a reliable spend
cap in that state. Existing per-instance enforcement limits also still apply.

## Verification

Tests use synthetic native Responses fixtures and local HTTP servers. A real
gateway/router plus PostgreSQL test exercises encrypted credential resolution,
stream persistence, reasoning/phase replay, tenant isolation and rejection of
an unsupported MCP request before history mutation. No test calls live OpenAI.

```sh
cargo test -p loom-provider-openai
cargo test -p loom-server --test openai
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

These prove local contract behavior, not live model output, account access,
production latency, real tool execution or pricing accuracy.

Official documentation checked 26 October 2026:

- [GPT-6 Luna](https://developers.openai.com/api/docs/models/gpt-6-luna)
- [Responses creation](https://developers.openai.com/api/reference/resources/responses/methods/create)
- [Responses streaming events](https://developers.openai.com/api/reference/resources/responses/streaming-events)
- [Function calling](https://developers.openai.com/api/docs/guides/function-calling)
- [Reasoning and stateless history](https://developers.openai.com/api/docs/guides/reasoning)
