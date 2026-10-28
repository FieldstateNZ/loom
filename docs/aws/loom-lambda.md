# Private Loom Lambda entry point

`cargo build --locked --release -p loom-server --bin loom-lambda` builds the
native entry point using AWS's `lambda_http` 1.3.2 runtime. The Dockerfile's
explicit `lambda` target packages `/var/runtime/bootstrap` with its matching
pinned Debian runtime and CA bundle. The final/default target remains the
ordinary server image. Do not copy this Debian-linked binary into a different
OS such as `provided.al2023`: a ZIP deployment needs a separately compatible
toolchain/build. This change provisions no AWS resources and replaces neither
the ordinary HTTP server nor explicit migration and provisioning jobs.

Build only from committed source, selecting the deployment architecture:

```sh
bash deploy/ci/docker_context.sh HEAD /tmp/owned-loom-context
docker build --platform linux/arm64 --target lambda -t loom-lambda:local /tmp/owned-loom-context
python3 deploy/ci/inspect_lambda_image.py loom-lambda:local --expected-arch arm64
```

The image inspector checks the actual Linux binary as UID 10001 on a read-only
filesystem, its CA bundle, fail-closed configuration, and two successive events
through the official Runtime API loop. The owned peer and runtime run on an
isolated internal Docker network; no host port is published. Liveness succeeds
and the admin route is refused without any database connection. The peer is a
local synthetic AWS envelope, not IAM verification, and no model is called.
This is ABI/startup/container evidence, not a cloud conversation or full memory
test. Cleanup removes only the inspector's uniquely named containers/network.
CI adds the same check for Linux AMD64 and never pushes these images.

The local ARM64 release build and image inspection pass with the pinned base's
Rust/Cargo 1.94.1. Both successive events complete as expected (200 liveness,
404 admin refusal), with zero database connections. This qualifies the ARM64
container's ABI and bounded initialization/runtime loop; memory conversations,
signed downstream requests and actual AWS freeze/thaw still need their own
deployment-scope evidence. The image is not a compatible Amazon Linux ZIP binary.

Require `LOOM_REQUEST_DRIVEN=true`, normal database/encryption/key configuration,
`LOOM_LAMBDA_ACCOUNT_ID`, `AWS_REGION` and `LOOM_LAMBDA_CALLER_ROLE_ARNS` (a finite
JSON array of same-account role ARNs without role paths). Pool capacity must be
at least two. Construction is lazy: no network, migration, polling or inference
occurs on initialization. The first admitted application request verifies the
exact release schema using SELECTs within its execution budget. Deploy migrations
separately with the existing operator command before enabling traffic.

## Private authorization

Deployment must use a Function URL with `AWS_IAM`. Grant only the intended caller
roles `lambda:InvokeFunctionUrl` with `lambda:FunctionUrlAuthType=AWS_IAM` and
`lambda:InvokeFunction` with `lambda:InvokedViaFunctionUrl=true`. Never grant
unrestricted direct invocation: caller-supplied IAM-shaped event metadata is not
cryptographic proof. AWS signing, actual policies and networking remain to be
implemented and qualified. Role/session/account, regional Function URL domain,
runtime function ARN and matching method/path receive additional local checks.

SigV4 owns outer `Authorization`. Carry the existing tenant virtual-key Bearer
in signed `x-tandem-authorization`; the adapter passes it to the unchanged tenant
authentication middleware as `Authorization`. It drops AWS signing headers,
host, forwarding claims and unknown headers. Cookies and queries are rejected.
Only health/readiness, whoami, MCP registration listing and stateful conversation,
turn and action routes are admitted. Admin, batch, stateless turns and OpenAPI
are excluded. Canonical UUID paths prevent alternate encoded routing.

## Response and cleanup

Requests are limited to 64 KiB and responses to 256 KiB. The adapter consumes
the existing router's full response inside an 18-second work budget and buffers
SSE bytes without translating them. It therefore supports bounded current turns,
not continuous voice streaming. Large history needs pagination before expanding
this private limit. The Lambda deadline bounds work and reserves two seconds for
cleanup, with an additional 100 ms margin before the runtime deadline.

One sandbox admits one invocation. Its supervisor keeps admission even if the
caller drops its future. A work timeout drops the real router/body future and
waits for conversation-lock and OAuth-lease rollback, registered partial-stream
usage/history settlement and actual SQLx connection release. Settlement is
registered synchronously before spawning; an unpolled task cannot look idle.
An aborted/panicking settlement poisons quiescence. Unresolved or failed cleanup
retires the process before acknowledgement, preventing unsafe freeze/reuse.

The adapter never retries inference or writes. A committed operation can still
have an ambiguous client result after process loss; retain its existing operation
identity and receipt/reconciliation behavior. Do not replay deleted memories.
Existing rate/spend admission is per process. Distributed admission and the
cheapest practical PostgreSQL host still require qualification before deployment.

Local tests cover actual official Runtime API conversion, boundary/header/size
checks, buffered SSE, abandoned callers and retirement; a real PostgreSQL/router
test cancels an actual partial provider stream and checks usage settlement,
closed database sessions and successful reconnect. Providers and Function URL
IAM metadata are simulated. These are not AWS cold/freeze/thaw, private routing,
complete Tandem memory, live inference or iPhone acceptance evidence.

References: [AWS Rust runtime](https://github.com/aws/aws-lambda-rust-runtime),
[Function URL events](https://docs.aws.amazon.com/lambda/latest/dg/urls-invocation.html),
[IAM conditions](https://docs.aws.amazon.com/lambda/latest/dg/urls-auth.html).

## Encrypted startup settings (opt in)

The native Lambda entrypoint can load one explicitly pinned SSM SecureString
before constructing its gateway or opening PostgreSQL connections. The ordinary
server, migration job and existing environment-based Lambda path are unchanged.
The caller policy is checked first. No global environment variable is mutated
after Tokio starts; validated secrets overlay a private configuration lookup.

Set only these non-secret references alongside the existing caller-role/pool
settings (the account/path/version here are synthetic):

```text
LOOM_BOOTSTRAP_PARAMETER_ARN=arn:aws:ssm:ap-southeast-2:123456789012:parameter/tandem/example/loom-runtime
LOOM_BOOTSTRAP_PARAMETER_VERSION=7
LOOM_LAMBDA_ACCOUNT_ID=123456789012
AWS_REGION=ap-southeast-2
LOOM_REQUEST_DRIVEN=true
```

The full ARN must match the configured account/region under `/tandem/`; versions
are explicit positive integers rather than movable labels. A partial reference,
non-Unicode value, or pre-existing `DATABASE_URL`, `LOOM_ROOT_ADMIN_TOKEN`,
`LOOM_ENCRYPTION_KEY` or `LOOM_KEY_PEPPER` causes refusal in this mode. This avoids
silently mixing credential sources. Missing ARN/version together retains the
established environment path.

The SecureString JSON is at most4096 UTF-8 bytes and has exactly these fields:

| Field | Value |
| --- | --- |
| `schema_version` | `1` |
| `kind` | `loom-private-lambda` |
| `database_url` | Existing restricted runtime PostgreSQL DSN with user/password/database, no URL query or fragment |
| `database_ca_pem` | One PEM CA trust certificate; never its private key |
| `database_ca_sha256` | SHA256 of the exact PEM bytes |
| `root_admin_token` | Existing random admin token,32–256 bytes without NUL/CR/LF |
| `encryption_key` | Existing64-hex credential-encryption key; no replacement/minting |
| `key_pepper` | Required explicit choice: `{"mode":"derived"}` preserves the existing binary derivation, or `{"mode":"explicit","value":"EXISTING_VALUE"}` preserves an existing text pepper |

The existing official AWS signer is shared with the private MCP client. It
retains its TRACE suppression and sensitive-header markings. Each request uses
only injected execution-role credentials, the fixed regional HTTPS dual-stack
SSM hostname and exact `GetParameter` bytes with `WithDecryption:true`. No
profile/metadata/SSO lookup, endpoint override, proxy, redirect or retry is used.
The whole DNS/TLS/response read is bounded to six seconds, including streaming
response limits. Metadata ARN/name/type/version and the strict document must
match. Errors expose no upstream object, parameter value, DSN or key.

The certificate fingerprint and PEM trust-anchor parsing are checked before
writing a new sandbox-private directory0700/file0600. SQLx receives only
`sslmode=verify-full` and that root-certificate path, preserving chain/hostname
verification. Its file guard stays alive for the runtime and removes owned
files on normal teardown; sandbox retirement also destroys `/tmp`. No startup
migration, polling or request budget changes are introduced. Warm invocations
reuse the pinned configuration, rather than rotating encryption keys beneath
existing data. Deploy a newly pinned version only through explicit operator
identity/recovery checks and runtime replacement.

Operator publication and least-privilege deployment remain work: runtime IAM
should permit `ssm:GetParameter` on the exact parameter ARN and any required
decryption for its key, with no parameter writes/history/path enumeration.
Never put the JSON, synthetic test configuration or real service keys in Git,
arguments, build contexts, logs or public CI artefacts.

Local evidence: [loom-lambda-bootstrap-evidence.json](loom-lambda-bootstrap-evidence.json).
The complete Rust workspace passes287 tests with two paid-provider tests ignored;
rustfmt and warnings-denied clippy pass. A clean functional-source ARM64 image
passes three actual Runtime API modes: existing environment, one independently
HMAC-verified TLS SSM handoff followed by200/404, and substituted-version exit
before invocation/database work. UID10001, read-only root and zero DB connections
hold. All owned fixture resources are removed. CI runs the same three modes on
AMD64. An initial local full run hit a PostgreSQL container SSL-startup failure
before application setup; its isolated recheck and complete two-thread run pass.
No production TLS relaxation/retry was introduced.

Separately, the existing Tandem/native Loom/new LucidBrain handler flow passes
four TLS/cleanup guards and14 memory checks with28 production migrations and
simulated inference. That flow uses existing environment settings, not SSM
bootstrap. Neither local proof qualifies real AWS IAM/KMS, private cloud database
routing, combined encrypted-startup memory or the iPhone. Those remain the next
deployment acceptance work. No paid host, provider test or phone update is made.

SSM contract: [GetParameter](https://docs.aws.amazon.com/systems-manager/latest/APIReference/API_GetParameter.html).
