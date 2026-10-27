-- Private recoverable token generations. Only ciphertext is stored here;
-- pending refresh operation + original pair are sealed together by the gateway.
CREATE TABLE mcp_private_credentials (
    server_id UUID PRIMARY KEY REFERENCES mcp_servers(id) ON DELETE CASCADE,
    revision BIGINT NOT NULL DEFAULT 1 CHECK (revision > 0),
    ciphertext BYTEA NOT NULL,
    nonce BYTEA NOT NULL CHECK (octet_length(nonce) = 12),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
