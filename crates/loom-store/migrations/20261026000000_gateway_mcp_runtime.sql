-- Opt-in private memory execution has a durable binding and uncertainty fence.
-- A crashed/interrupted run stays running and cannot silently be re-executed.
CREATE TABLE gateway_mcp_runtime (
    conversation_id UUID PRIMARY KEY REFERENCES conversations(id) ON DELETE CASCADE,
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    fingerprint TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'idle' CHECK (status IN ('idle', 'running')),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX gateway_mcp_runtime_tenant_idx ON gateway_mcp_runtime(tenant_id);
