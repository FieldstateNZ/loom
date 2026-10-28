# Signed private MCP transport

Loom uses `loom_mcp::http::PrivateHttpClient` for MCP messages, context
attestation and recoverable OAuth renewal. A canonical regional HTTPS Function
URL ending in `/mcp` selects AWS SigV4; ordinary HTTPS and loopback HTTP retain
the existing direct transport. The Function URL region must equal `AWS_REGION`.
No request can change the configured origin, add a query/fragment or follow a
redirect. Only the existing MCP/context/token paths and methods are admitted.

The official AWS signer uses service `lambda` and the current execution-role
`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` and `AWS_SESSION_TOKEN` on each
request. All three must be present. No credential is obtained from a phone,
model, metadata service or interactive developer login. There is no credential
lookup on client construction or a background credential worker.

SigV4 owns outer `Authorization`; the existing application Bearer moves to the
signed, sensitive `x-tandem-authorization` header. All four expected principal,
workspace, agent and credential headers remain signed and unchanged. Token
renewal retains its original request body and pending operation, and needs no
application Bearer. Its encrypted credentials, leases and committed receipt
validation remain in the existing private-account service.

The wrapper delegates MCP serialization, response/SSE parsing, session headers,
explicit event limits and error handling to the pinned RMCP reqwest client.
Signing uses exactly the SDK's serialized JSON bytes and headers. The original
3-second connect/5-second total HTTP deadlines, redirect refusal and no-retry
policy remain. Function URL payloads are limited to 64 KiB. The default SDK SSE
limit is preserved; callers' explicit limits are passed through unchanged.

The signer emits request details at TRACE, including raw custom headers. Its
synchronous signing operation therefore runs under a scoped no-op tracing
dispatcher. Other application tracing stays configured normally. Signing
failures use non-diagnostic errors, and signing/application credentials are
marked sensitive. Do not add raw signer/request debugging when troubleshooting.

Deployment must separately configure the target Function URL as `AWS_IAM` and
grant the intended Loom execution role only the exact target function:
`lambda:InvokeFunctionUrl` conditioned on `lambda:FunctionUrlAuthType=AWS_IAM`
and `lambda:InvokeFunction` conditioned on `lambda:InvokedViaFunctionUrl=true`.
Never grant unrestricted direct invocation. The target validates its allowed
caller and the existing application identities; this transport provisions no
IAM policy, function or paid resource.

## Local evidence and limits

Five wire tests independently verify HMAC, body hash, temporary token, context
identities and renewal operation bytes. They exercise the actual RMCP parser,
GET/DELETE/session behavior, errors without retries, destination/size/redirect
refusal and TRACE redaction. Synthetic signing credentials and a loopback
signing target are injected only inside the Rust test module. Removing the TRACE
guard reproduces application-Bearer disclosure in the regression.

The full workspace passes 283 tests (two paid/live tests remain ignored),
format checking and all-targets Clippy with warnings denied. These are component
and direct-transport regressions, not a complete signed Tandem/Loom/LucidBrain
conversation or actual AWS IAM, TLS/DNS and network proof. The existing local
signed Tandem-to-Loom memory report still uses unsigned loopback HTTP for this
second hop. Linux deployment packaging, complete signed-hop qualification,
practical database hosting/cost and existing-iPhone acceptance remain separate.
No paid provider request or AWS change is part of these checks.

The signer dependencies are pinned. AWS transitive versions in `Cargo.lock`
retain compatibility with the repository's Rust 1.94.0 container toolchain:
runtime-api 1.12.3, types 1.4.9, async 1.2.14 and runtime-api-macros 1.0.0.
Updating them must preserve that compiler requirement; a newer local compiler
alone does not establish the deployment build's compatibility.
