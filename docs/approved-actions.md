# Confirmed task actions for Tandem

This is an opt-in extension to Loom's owned `/v1` conversation runtime. The
default private-memory mode remains read-only. The application must add a visible
confirmation control before enabling task proposals for a user. These routes do
not establish that the iOS controls, Cognito binding or a production deployment
have been integrated.

Loom owns the proposal, approval, operation UUID and execution state. LucidBrain
owns the actual task and its atomic capture receipt. Inference can call only
`propose_task` with a title and optional detail. It never receives the private
`capture_item` tool, an operation-ID argument, approval/dispatch routes, MCP URL
or credential. User text such as "approved" is not a decision on this API.

## Operator and application contracts

Use the identity and expected-request headers from `private-memory.md`. Require
LucidBrain's `capture_operation_version: 1` contract and its reviewed
`capture_item`/`get_capture_operation` implementation. Apply the additive
`20261027000000_gateway_actions.sql` migration before enabling this mode.

The private registration contains an explicitly chosen real-world category:

```json
{
  "enabled": true,
  "allowed_tools": ["memory_search", "list_categories", "capture_item", "get_capture_operation"],
  "gateway": {
    "kind": "lucidbrain_memory",
    "binding": {
      "principal_id": "PRINCIPAL_UUID", "workspace_id": "WORKSPACE_UUID",
      "agent_definition_id": "ASSISTANT_UUID", "credential_id": "CREDENTIAL_UUID"
    },
    "task_capture": {
      "workspace_slug": "my-workspace", "category_id": "CATEGORY_UUID",
      "provenance": "real_world"
    }
  }
}
```

The gateway verifies actual category metadata in the pinned workspace. The
model cannot choose a category, workspace, delegate, schedule, operation UUID or
provenance. Category/registration changes invalidate an undispatched approval.
Both the application's `gateway_mcp` request and operator task policy must
explicitly declare real-world provenance. A simulated request cannot propose a
real-world task. This is an action boundary, not a complete simulation-memory
integration.

The private application requests the reviewed memory subset and proposals:

```json
{
  "content": [{"type": "text", "text": "Please create a task to call the builder."}],
  "stream": true,
  "gateway_mcp": {
    "name": "lucidbrain", "allowed_tools": ["memory_search"],
    "expected_binding": {
      "principal_id": "PRINCIPAL_UUID", "workspace_id": "WORKSPACE_UUID",
      "agent_definition_id": "ASSISTANT_UUID", "credential_id": "CREDENTIAL_UUID"
    },
    "propose_tasks": true,
    "provenance": "real_world"
  }
}
```

`GET /v1/mcp-servers` advertises `gateway_action_version: 1`. Refuse an older
gateway before enabling proposals. The provider-hosted `withMcp` client helper
does not enable this extension; use the generated owned-conversation API shapes.

The decision authority is the authenticated tenant virtual key. Tandem must keep
it on its backend, select a unique user binding from the verified Cognito subject,
and forward only an actual application confirmation. A shared broad tenant key
is not per-user authorization. Loom records the deciding virtual-key UUID and
time; it does not independently authenticate a Cognito user or treat a caller's
`approved_by` text as evidence.

## Lifecycle

1. The model proposes text. Loom validates the whole batch before execution,
   allocates the action/operation UUID, and persists an immutable proposal and
   approval digest. The function result says `requires_application_approval` and
   `created: false`. SSE emits `loom_action_proposed` with the public card data.
2. The application fetches `GET /v1/conversations/{id}/actions`. Show the exact
   title, full detail, category and expiry from that response. A confirmation
   submits the returned digest to `POST .../actions/{action}/decision` with
   `{ "decision": "approve", "approval_digest": "..." }`. `reject` ends a pending
   proposal. Approval alone performs no task write.
3. `POST .../actions/{action}/dispatch` with `{}` revalidates the registration,
   credential identity, current tool policy, category and approval digest. A
   conditional committed state change from `approved` to `dispatched` arbitrates
   with cancellation and expiry before any capture call. Only then does Loom
   send the immutable payload and original operation UUID to LucidBrain once.
4. A verified committed receipt changes the action to `completed` and adds one
   application-confirmed result message to conversation history in the same
   PostgreSQL transaction. HTTP 200 alone is not success: the client must require
   `completed` and a committed receipt before speaking a creation confirmation.
5. A lost response, timeout or stopped execution stays `dispatched` or
   `uncertain`. `POST .../reconcile` is read-only, using `get_capture_operation`.
   A committed receipt completes the original action. `not_observed` is only a
   point-in-time absence; it does not authorize a new operation ID.
6. If the original approval is still valid and the user explicitly requests
   another attempt, `POST .../retry` sends exactly that operation UUID and payload.
   It is allowed only from `uncertain`, before expiry, without a cancellation
   request, and at most three dispatch attempts total. No model call or automatic
   write retry occurs in any action command.

The proposal/approval lifetime is fifteen minutes. Expiry prevents new dispatch
and retry; it never hides an already-sent operation or a committed receipt. At
most one task can be proposed per model turn and fifty per conversation.

`POST .../cancel` can atomically cancel a pending or approved undispatched task.
After dispatch it sets `cancel_requested` without claiming the task was undone.
It prevents another write attempt, while read-only reconciliation remains
available. A result may correctly be `completed` with `cancel_requested: true`.

Commands accept no payload/identity overrides. The digest includes the tenant,
conversation, operation UUID, execution fingerprint and full proposal. It is
recomputed before approval and dispatch, not merely compared with a cached copy.
Receipt lookup can follow intentional credential replacement and removal of
write permission, provided principal, workspace, assistant, registration and
endpoint remain the same. It requires only receipt-read permission.

## Durability, concurrency and deletion

State is PostgreSQL-owned. Conversation advisory transaction locks prevent
overlapping model turns and dispatch/reconciliation commands across replicas.
Dispatch also holds the operator MCP registration row against concurrent
reconfiguration. Cancellation uses conditional row updates so it can still win
while dispatch is verifying its connection. Dropping a future or restarting a
process cannot reset a dispatched operation to approved.

These action records are distinct from `pending_tool_calls`: that existing store
records blocking calls on provider-managed sessions and accepts provider tool
results. It is not an application approval or delivery ledger. The new action
runtime reuses the existing conversation history, tenant boundary, encryption,
MCP registration and connection implementation; it does not invent another
provider-managed session lifecycle.

Deleting a conversation is serialized with execution and refused while a live
approval or unconfirmed dispatched action needs attention. Undispatched pending
proposals can be discarded with their conversation; completed tasks remain owned
by LucidBrain. Tandem must preserve access to action reconciliation when upstream
deletion is refused, rather than hiding the local session first.

## Current limits

An unconfirmed canceled dispatch with no visible receipt can remain uncertain.
LucidBrain v1 has no authoritative no-effect/cancel tombstone for an operation
that might still be executing. The gateway does not manufacture that evidence,
retry a canceled write, or delete its unresolved record. This liveness case needs
a downstream operation cancellation/fencing contract before claiming complete
action reconciliation.

Only task inbox capture is implemented. Memory writes, scheduling, delegation,
device actions and external messages are not offered. Full Tandem API/mobile
confirmation wiring, broader identity provisioning, AWS hosting, live inference
and device validation remain separate application work. Keep this feature opt-in
until that integration has been verified.
