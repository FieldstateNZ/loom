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
