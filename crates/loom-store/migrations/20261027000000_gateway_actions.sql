-- Application approvals are separate from provider-managed session tool calls.
-- Operation IDs are gateway-owned and remain immutable across dispatch/retry.
CREATE TABLE gateway_actions (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    conversation_id UUID NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    tool_use_id TEXT NOT NULL,
    payload JSONB NOT NULL,
    digest TEXT NOT NULL,
    request_policy JSONB NOT NULL,
    fingerprint TEXT NOT NULL,
    registration_id UUID NOT NULL,
    endpoint_hash TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','approved','rejected','canceled','expired','dispatched','uncertain','completed')),
    approved_by_key_id UUID,
    approved_at TIMESTAMPTZ,
    cancel_requested BOOLEAN NOT NULL DEFAULT false,
    attempts INTEGER NOT NULL DEFAULT 0,
    receipt JSONB,
    expires_at TIMESTAMPTZ NOT NULL DEFAULT now() + interval '15 minutes',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(conversation_id,tool_use_id),
    CHECK (status NOT IN ('approved','dispatched','uncertain','completed') OR approved_at IS NOT NULL),
    CHECK ((status = 'completed') = (receipt IS NOT NULL))
);
CREATE INDEX gateway_actions_conversation_idx ON gateway_actions(tenant_id,conversation_id,created_at,id);
