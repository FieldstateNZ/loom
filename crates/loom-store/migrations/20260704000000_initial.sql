-- Loom initial schema.
--
-- Every tenant-owned table carries an explicit `tenant_id` so the repository
-- layer can scope every read and write; the store never exposes an unscoped
-- accessor for tenant data. Conversation history is stored as JSONB carrying
-- the exact loom-core domain model, so replaying a conversation back to a
-- provider is lossless.

-- Tenants -------------------------------------------------------------------
CREATE TABLE tenants (
    id         UUID PRIMARY KEY,
    slug       TEXT NOT NULL UNIQUE,
    name       TEXT NOT NULL,
    status     TEXT NOT NULL DEFAULT 'active',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Virtual keys --------------------------------------------------------------
CREATE TABLE virtual_keys (
    id                  UUID PRIMARY KEY,
    tenant_id           UUID NOT NULL REFERENCES tenants (id) ON DELETE CASCADE,
    key_hash            TEXT NOT NULL UNIQUE,
    key_prefix          TEXT NOT NULL,
    name                TEXT NOT NULL,
    status              TEXT NOT NULL DEFAULT 'active',
    scopes              JSONB NOT NULL DEFAULT '[]'::jsonb,
    budget_limit_amount NUMERIC,
    budget_window       TEXT,
    budget_action       TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at        TIMESTAMPTZ
);

-- Tenant lookups; key_hash lookups use the UNIQUE index created above.
CREATE INDEX idx_virtual_keys_tenant ON virtual_keys (tenant_id);

-- Provider credentials ------------------------------------------------------
-- A NULL tenant_id denotes a gateway-global credential. The unique constraint
-- uses NULLS NOT DISTINCT (PostgreSQL 15+) so at most one global credential
-- exists per provider.
CREATE TABLE provider_credentials (
    id               UUID PRIMARY KEY,
    tenant_id        UUID REFERENCES tenants (id) ON DELETE CASCADE,
    provider         TEXT NOT NULL,
    encrypted_secret BYTEA NOT NULL,
    nonce            BYTEA,
    aad              BYTEA,
    base_url         TEXT,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT uq_provider_credentials_tenant_provider
        UNIQUE NULLS NOT DISTINCT (tenant_id, provider)
);

CREATE INDEX idx_provider_credentials_tenant ON provider_credentials (tenant_id);

-- Conversations -------------------------------------------------------------
-- `system` is stored alongside `metadata` so the full loom-core Conversation
-- aggregate round-trips losslessly.
CREATE TABLE conversations (
    id         UUID PRIMARY KEY,
    tenant_id  UUID NOT NULL REFERENCES tenants (id) ON DELETE CASCADE,
    provider   TEXT NOT NULL,
    model      TEXT NOT NULL,
    system     TEXT,
    metadata   JSONB NOT NULL DEFAULT 'null'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_conversations_tenant ON conversations (tenant_id);

-- Messages ------------------------------------------------------------------
-- `content` and `usage` hold the exact JSON serialisation of loom-core's
-- Vec<ContentPart> and Usage respectively.
CREATE TABLE messages (
    id                   UUID PRIMARY KEY,
    conversation_id      UUID NOT NULL REFERENCES conversations (id) ON DELETE CASCADE,
    seq                  INTEGER NOT NULL,
    role                 TEXT NOT NULL,
    content              JSONB NOT NULL,
    raw_provider_payload JSONB,
    usage                JSONB,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT uq_messages_conversation_seq UNIQUE (conversation_id, seq)
);

-- Conversation history read path: ordered by (conversation_id, seq).
CREATE INDEX idx_messages_conversation_seq ON messages (conversation_id, seq);

-- Usage events --------------------------------------------------------------
CREATE TABLE usage_events (
    id                 UUID PRIMARY KEY,
    tenant_id          UUID NOT NULL REFERENCES tenants (id) ON DELETE CASCADE,
    virtual_key_id     UUID REFERENCES virtual_keys (id) ON DELETE SET NULL,
    conversation_id    UUID REFERENCES conversations (id) ON DELETE SET NULL,
    provider           TEXT NOT NULL,
    model              TEXT NOT NULL,
    input_tokens       BIGINT NOT NULL DEFAULT 0,
    output_tokens      BIGINT NOT NULL DEFAULT 0,
    cache_read_tokens  BIGINT NOT NULL DEFAULT 0,
    cache_write_tokens BIGINT NOT NULL DEFAULT 0,
    server_tool_counts JSONB NOT NULL DEFAULT '{}'::jsonb,
    cost               NUMERIC,
    raw_usage          JSONB,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Usage rollups by tenant over time.
CREATE INDEX idx_usage_events_tenant_created ON usage_events (tenant_id, created_at);
