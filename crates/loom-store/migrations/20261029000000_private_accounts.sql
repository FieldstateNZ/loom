-- Stable initial tenant/key ownership. Secrets stay in the operator's durable
-- private input; only the existing peppered key hash is stored in virtual_keys.
-- Keep this receipt after revocation: a retry must never resurrect an account.
CREATE TABLE private_accounts (
    namespace TEXT PRIMARY KEY CHECK (namespace ~ '^tandem-[0-9a-f]{40}$'),
    operation_id UUID NOT NULL UNIQUE,
    request_hash TEXT NOT NULL CHECK (request_hash ~ '^[0-9a-f]{64}$'),
    tenant_id UUID NOT NULL UNIQUE REFERENCES tenants(id) ON DELETE RESTRICT,
    key_id UUID NOT NULL UNIQUE REFERENCES virtual_keys(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
