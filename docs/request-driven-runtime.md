# Request-driven runtime preparation

`LOOM_REQUEST_DRIVEN=true` defaults to two database slots, no startup migrations
and no batch polling. Explicit conflicting migration/worker settings fail at
startup. Exact-release verification still uses SELECTs before HTTP is admitted.
Dedicated `--migrate` remains an explicit separate operator command. Ordinary
server defaults are unchanged when this flag is absent.

The runtime pool discards each database connection when its checkout ends. Held
conversation locks/transactions retain their connection until their normal
cleanup completes; no idle socket is intentionally retained for a later request.
This trades additional database handshakes for predictable idle behavior in a
small personal pilot. Pool admission still reserves half the local pool for
non-lock work; it is not a distributed rate/budget limit or host capacity proof.

SQLx drop/rollback/close runs asynchronously. A Lambda invocation adapter must
wait for response and transaction cleanup before it permits a freeze. This pool
policy alone does not implement that barrier, a Lambda handler, signed internal
IAM transport, restricted routing or actual AWS connectivity. Aurora auto-pause
and AWS freeze/thaw remain unqualified. Do not deploy based on this flag alone.

The real-PostgreSQL regression checks backend disconnection, fresh sessions on
later checkouts, rollback/lock release, committed receipt survival, SQL errors
and unchanged ordinary-server connection reuse. No paid model call is used.
