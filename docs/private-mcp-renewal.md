# Private LucidBrain credential renewal

This gateway release implements the private recoverable grant from LucidBrain
PR #134. It is opt-in per registration. A legacy static credential retains its
existing behavior until a root operator installs the renewable pair. Migration
jobs must apply the new `mcp_private_credentials` table and reconcile runtime
DML grants first. No startup/provider/phone code installs or exports the pair.

Create the reviewed `lucidbrain_memory` registration normally, then send a private
JSON body to the root-admin-only endpoint:

`PUT /admin/tenants/{tenant}/mcp-servers/{name}/private-credential`

Fields are `client_id`, the four-UUID `binding`, the exact `external_account`
attestation, `access_token`, `refresh_token`, `scope` (space-separated OAuth
scopes), `access_expires_at` and `refresh_expires_at`. For the current private
account job, scopes are `items:read items:write`. Use the actual job receipt and
live account evidence; these are backend secrets, never phone/model input. The
endpoint only accepts HTTPS `/mcp` or exact loopback HTTP for local checks. Its
OAuth endpoint is derived from the same origin as `/api/v1/oauth/token`, with no
redirects. It verifies the actual authenticated MCP context, negotiated renewal
version 1, identity, external account and registered finite tool set. Initial
access must have more than 120 seconds left. The response is metadata only.
An identical unrotated installation retry is idempotent; it cannot reset a
pending/rotated generation.

The gateway seals the complete private state with its existing AES-GCM key. AAD
binds tenant, registration UUID and name. The sealed state also pins the endpoint
and complete registration policy, scope, binding/account, current token pair and
pending random UUIDv4. Retain the gateway encryption key and LucidBrain recovery
key/version with coordinated backups. There is no key ring or implicit recovery
from deleted/corrupt/wrong-key records.

Before a logical conversation, task dispatch or receipt lookup, the gateway
admits another database lock slot and resolves the private generation. It uses
an unexpired token only when more than 120 seconds remain. Otherwise it acquires
an exclusive PostgreSQL session advisory lease, commits the pending operation
before HTTP, and calls the private grant once. Unknown HTTP or local success-save
outcomes retain the original operation and old refresh secret. A later request
or gateway restart replays that operation; it never tries ordinary OAuth or a
new operation to guess the outcome. Returned IDs, account, scopes, tokens, state
and absolute/relative deadlines are validated before an atomic revision-CAS
save. A recovered expired generation is saved first and can be renewed once
more; there is no unbounded loop. A newly returned active token with insufficient
lifetime is saved but fenced for operator recovery.

The client owns a shared advisory lease through its actual transport lifetime;
renewal cannot revoke credentials beneath active clients in any gateway replica.
A conflicting lease returns unavailable before inference or action dispatch.
No tool/write is transparently retried. Conversation fingerprints, task approvals
and original action operation IDs do not change on token renewal. Normal registry
replacement is rejected while private state exists. Controlled replacement,
external account disablement and erasure remain explicit lifecycle work; deleting
an idle registration cascades its private row. Private registrations cannot be
forwarded into provider-hosted MCP resolution.

Each renewable conversation uses two admitted long-lived connections (conversation
serialization and credential lease). Admission shares the existing floor(pool/2)
slot budget, preserving normal database work capacity. A four-connection pool
admits one renewable conversation; a ten-connection pool admits at most two.
Locks are connection-owned, no transaction spans renewal HTTP, and connections
close on drop. Normal async cleanup retains admission until close finishes.
Stalled streaming responses may hold a lease until their transport drops; ingress
and client request deadlines remain operational requirements. This is a personal
pilot design, not a qualified high-concurrency service.

Local checks exercise real PostgreSQL, migrations, encrypted storage, gateway
HTTP, restart and both memory/task gateway paths. The focused Rust protocol
peers simulate AI, renewal and MCP responses. Separate joint-service evidence
must name exact Loom/LucidBrain/Tandem source revisions; Rust protocol doubles
alone do not prove the actual deployed service contract. AWS activation, retained
key restore, cross-process crash/replica load, continuous Cognito disablement,
recoverable tenant/key creation, actual iPhone and live Claude/OpenAI speech
remain separate gates. No real provider credits are needed for these checks.
