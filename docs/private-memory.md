# Private LucidBrain memory execution

This is an opt-in Loom-owned execution path for the current persistent `/v1`
conversation API. Tandem can use OpenAI function calling to read LucidBrain
memory while the MCP connection stays inside the application's network.
Provider-hosted MCP is a separate path; no private server URL or bearer token is
forwarded to OpenAI. This does not claim the full OASP AgentProvider lifecycle,
approved writes, task idempotency, live-provider qualification or AWS deployment.

It depends on LucidBrain's credential-context and request-binding implementation
in [LucidBrain PR #130](https://github.com/FieldstateNZ/lucidbrain/pull/130),
revision `6ab7166aed47014038512f68509af2e00b732014`. The OpenAI adapter is
the prerequisite in [Loom PR #48](https://github.com/FieldstateNZ/loom/pull/48).

## Registration and identity

An operator registers the existing tenant-scoped encrypted MCP credential using
`PUT /admin/tenants/{tenant}/mcp-servers/lucidbrain`:

```json
{
  "url": "https://your-private-lucidbrain.example/mcp",
  "authorization_token": "PRIVATE_ASSISTANT_ACCESS_TOKEN",
  "tool_configuration": {
    "enabled": true,
    "allowed_tools": ["memory_search"],
    "gateway": {
      "kind": "lucidbrain_memory",
      "binding": {
        "principal_id": "LUCIDBRAIN_PRINCIPAL_UUID",
        "workspace_id": "PINNED_WORKSPACE_UUID",
        "agent_definition_id": "LUCIDBRAIN_AGENT_UUID",
        "credential_id": "STABLE_CREDENTIAL_UUID"
      }
    }
  }
}
```

These placeholders deliberately cannot authenticate. A gateway registration
cannot be used through the legacy provider-hosted MCP resolver; it fails before
forwarding its token. Only an operator can register or replace the binding.
HTTPS is required except loopback HTTP for local integration. URLs with userinfo,
query parameters or fragments are refused, as are redirects. Gateway networking,
DNS and trusted certificates must actually reach this endpoint; registration
does not configure a VPC or prove connectivity.

The acting principal is a LucidBrain user, normally an agent service identity.
It is not a Cognito subject. Tandem's private provisioning record must establish
which user may use that principal/workspace/assistant/credential combination.
The first pilot continues to use a distinct Loom tenant and key per Tandem user;
this path does not introduce end-user principals inside a shared Loom tenant.

Before inference, Loom calls `/mcp/context` with the registered credential,
matches all four IDs, requires schema version 1 and stateless transport, and
requires an explicit finite policy containing only `memory_recall` and/or
`memory_search`. An unrestricted (`null`) tool policy is refused. It discovers
the actual schemas using the official Rust MCP SDK, the legacy 2025-03-26
handshake, and JSON/SSE Streamable HTTP. The discovered names must match the
reported policy. Only the requested, registered subset is offered to the model.

All MCP HTTP requests carry LucidBrain's four expected-identity headers. The
server checks them against the same request's fresh authentication context.
Reassigning a credential's assistant/workspace after preflight therefore denies
later requests, including those on an existing connection. Each actual read also
rechecks context. No assertion supplied by a model or mobile request can change
the operator registration or credential identity.

## Conversation API

`GET /v1/mcp-servers` adds `gateway_memory_version: 1`. A client must check it
before relying on private execution: an older server may ignore unknown JSON
fields. Tandem refuses that silent downgrade.

Start a fresh persistent conversation, then send:

```json
{
  "content": [{"type": "text", "text": "What tea do I prefer?"}],
  "stream": true,
  "options": {"max_tokens": 700},
  "gateway_mcp": {
    "name": "lucidbrain",
    "allowed_tools": ["memory_search"],
    "expected_binding": {
      "principal_id": "LUCIDBRAIN_PRINCIPAL_UUID",
      "workspace_id": "PINNED_WORKSPACE_UUID",
      "agent_definition_id": "LUCIDBRAIN_AGENT_UUID",
      "credential_id": "STABLE_CREDENTIAL_UUID"
    }
  }
}
```

The expected binding is a caller assertion that must match the operator's
registration and authenticated peer; it never grants permissions. Tandem
constructs it from private configuration, not the mobile chat body. No URL or
token is accepted in `gateway_mcp`. Mixing client-defined tools, provider-hosted
MCP, provider-executed tools or native provider-option bags is refused. Initial
user turns must be text, not caller-supplied tool results. Stateless execution is
explicitly rejected because it cannot supply the durable execution fence.

## Execution and recovery

Loom serializes stateful turns across replicas using a PostgreSQL advisory
transaction lock held until the response is consumed or dropped. This includes
ordinary turns, so a caller cannot overlap a private run with a legacy turn.
The lock consumes one database connection for the response lifetime; capacity
planning must account for that plus normal queries. Other simultaneous turns
receive 409 instead of racing history writes.

The additive `gateway_mcp_runtime` table pins the registration URL/ID, identity
and tool policy to the conversation. Access-token refresh under the same
credential ID can continue. Changing the binding or tool subset requires a new
conversation. A private conversation cannot bypass verification by omitting
`gateway_mcp` on a later turn.

The runtime marks a run `running` before persisting the user message or calling
the model. It persists each intermediate assistant tool call and each actual
tool result. Native reasoning and function-call metadata in normalized parts
survive subsequent model requests. Final success is emitted only after the final
assistant message is stored and the runtime returns to `idle`.

Crashes, cancellation, database errors or uncertain upstream outcomes leave the
durable fence set. Later calls are refused, including after a process restart.
There is no transparent retry or false exactly-once claim; the current recovery
is a new conversation. Writes remain disabled until durable operation IDs,
approval and outcome reconciliation are implemented.

The whole model-issued call set is checked before any call executes: names must
be offered reads, IDs must not repeat, and host-side validation enforces the
source-reviewed argument types, bounds and fields. Workspace/URL arguments and
unreviewed schema extensions are not executable. Reads have bounded results;
provider/server errors are surfaced generically without raw diagnostic bodies.
Sampling, elicitation/input-required continuations, asynchronous MCP tasks and
write tools are unsupported. The SDK's HTTP/SSE retries and session recovery are
disabled. Request timeouts and cancellation do not prove a remote read stopped.

At most four model passes and four memory calls run per logical turn, within a
60-second execution deadline. Model output defaults to 700 tokens per pass,
with a 2,000-token maximum. Rate limits and budgets are rechecked before every
subsequent inference. Each pass records its own usage/cost; logical response
usage and cost are the sum of the passes. Missing pricing remains unknown, not a
zero-cost assertion.

The SSE path sends one logical `turn_started`, safe `loom_memory_activity`
events, and one final answer/`turn_ended`, with five-second keepalive comments.
Activity includes only tool name and state. Intermediate reasoning, tool
arguments and memory payloads are not emitted as spoken answer text. Final
answer parts are buffered until the provider pass is validated and persisted;
this milestone does not provide live final-answer token deltas or replayable
OASP session events. Streamed messages retain the existing `raw = null`
limitation; provider replay metadata is retained in the normalized parts.

## Validation

`cargo test --workspace --all-features` covers the official MCP client against
protocol fixtures and a real Loom router/PostgreSQL migration path with mock
OpenAI/MCP peers. Cases include encrypted credentials, retained history,
multi-pass usage, workspace/binding denial before inference, write-tool denial,
bounded loops, concurrent turns, cancellation/drop and durable failure fences.

Tandem additionally supplies `Scripts/test-private-memory-integration.mjs`, which
uses actual LucidBrain Express/MCP and PostgreSQL with synthetic users/memories,
the real Loom binary and Tandem HTTP/SQLite. Only OpenAI Responses is mocked.
Its report records exact source commits and separates that evidence from live
AI, semantic search, device behavior, production provisioning and deployment.
