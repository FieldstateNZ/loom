# Atomic initial Tandem account setup

`PUT /admin/private-accounts/{namespace}` is an operator-only initial tenant/key
path. It uses the existing root admin authentication. It does not verify Cognito
tokens, provision LucidBrain, install MCP/provider credentials or activate a
phone. The Tandem operator helper performs the external identity checks.

The request has exactly `schema_version: 1`, `issuer`, case-sensitive `subject`,
UUIDv4 `operation_id` and `key`. The operator creates the key from 32 random bytes,
encodes it as canonical unpadded base64url with `loom_test_` or `loom_live_`, and
**durably saves the request before sending it**. This path accepts an operator
proposed secret instead of generating a one-time-response secret on the server.

The namespace preserves Tandem's existing format:
`tandem-` plus the first 40 lowercase SHA-256 hex characters of UTF-8
`JSON.stringify([issuer, subject])`. The server computes and checks it. Neither
issuer nor subject is stored as plaintext in the new receipt. Namespace hashes
are stable pseudonymous identifiers, not anonymous data.

The server stores only the existing peppered key hash. A bounded PostgreSQL
transaction serializes the namespace across replicas and atomically inserts the
tenant, virtual key and immutable operation receipt. A crash before commit leaves
none of those rows. A lost commit reply is reconciled by replaying the exact
original operation and secret. Concurrent identical calls return one tenant/key;
one returns 201 and the rest 200 with the same receipt. Responses are `no-store`
and contain identifiers, a display prefix and the original timestamp, never the
secret. The request body is limited to 8 KiB and is not Debug-logged.

Replay requires the original operation/digest, active owning tenant, original
key hash/prefix/tenant and active key. Changed requests, re-used key hashes,
operations already used for another namespace and pre-existing legacy slugs are
conflicts. Conflict paths roll back all candidate rows. Revocation or suspension
never creates a successor or reactivates a row. Existing key policy is not reset.
The receipt has restrictive foreign keys and remains an initial ownership fence.
Controlled replacement, erasure lifecycle and coordinated backup/restore need
their own explicit operations; this endpoint is not a replacement mechanism.

If the durable pending file is lost, the server cannot reconstruct the original
key. Preserve it through initial handoff and secure recovery under the operator's
private-file/backup policy. Do not generate a new operation to work around a
conflict. Changing the key pepper also changes lookup hashes; retained pepper and
database state must be restored together.

The new migration follows private MCP renewal and is required before this server
release starts under schema verification. Runtime roles need the same restricted
table DML rights used by existing admin operations, without DDL/migration rights.
The actual-service qualification exercises those roles separately from the
router tests. No real AWS resources or account bindings are activated by this PR.
