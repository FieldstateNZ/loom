-- Tenant-scoped MCP server registry.
--
-- Conversations reference an MCP server by name; the gateway resolves the row
-- at request time, loads the URL, and decrypts the authorization token to
-- inject it into the provider request server-side. The token is encrypted at
-- rest with the same envelope-encryption pattern as provider_credentials: the
-- ciphertext is bound (via AEAD associated data derived from the row identity)
-- to its `(tenant_id, name)` pair, so a ciphertext relocated into another row
-- fails to decrypt. `encrypted_token`/`nonce` are NULL when the server needs
-- no authorization.
CREATE TABLE mcp_servers (
    id                 UUID PRIMARY KEY,
    tenant_id          UUID NOT NULL REFERENCES tenants (id) ON DELETE CASCADE,
    name               TEXT NOT NULL,
    url                TEXT NOT NULL,
    encrypted_token    BYTEA,
    nonce              BYTEA,
    tool_configuration JSONB,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT uq_mcp_servers_tenant_name UNIQUE (tenant_id, name)
);

CREATE INDEX idx_mcp_servers_tenant ON mcp_servers (tenant_id);
