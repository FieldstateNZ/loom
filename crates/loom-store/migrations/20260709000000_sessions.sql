-- Conversation ≠ Session (OASP Wave 1).
--
-- OASP's structural insight is that a Conversation is not a Session: the durable
-- thread a user cares about outlives the disposable provider execution contexts
-- it runs on. A Session is one such context. A Conversation has exactly one
-- *active* session (conversations.current_session_id) and keeps any superseded
-- sessions as lineage — derived as its other sessions, oldest-first.
--
-- This migration is additive: existing conversations each gain one implicit
-- active session that adopts their whole history, so the split is transparent to
-- existing data and the /v1 surface keeps working unchanged.

-- Sessions ------------------------------------------------------------------
CREATE TABLE sessions (
    id              UUID PRIMARY KEY,
    conversation_id UUID NOT NULL REFERENCES conversations (id) ON DELETE CASCADE,
    tenant_id       UUID NOT NULL REFERENCES tenants (id) ON DELETE CASCADE,
    status          TEXT NOT NULL DEFAULT 'active',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Lineage read path: a conversation's sessions, oldest-first.
CREATE INDEX idx_sessions_conversation ON sessions (conversation_id, created_at);
CREATE INDEX idx_sessions_tenant ON sessions (tenant_id);

-- A Conversation points at its current (active) Session. Nullable so the
-- create transaction can insert the conversation, then its session, then link
-- them (the two tables reference each other); every live conversation has one.
ALTER TABLE conversations
    ADD COLUMN current_session_id UUID REFERENCES sessions (id);

-- Every message is recorded against the Session that produced it.
ALTER TABLE messages
    ADD COLUMN session_id UUID REFERENCES sessions (id);

CREATE INDEX idx_messages_session ON messages (session_id);

-- Backfill: one implicit active session per existing conversation, adopting its
-- whole history. No-op on an empty database (e.g. a fresh test container).
WITH new_sessions AS (
    INSERT INTO sessions (id, conversation_id, tenant_id, status, created_at)
    SELECT gen_random_uuid(), c.id, c.tenant_id, 'active', c.created_at
    FROM conversations c
    RETURNING id, conversation_id
)
UPDATE conversations c
SET current_session_id = ns.id
FROM new_sessions ns
WHERE ns.conversation_id = c.id;

UPDATE messages m
SET session_id = c.current_session_id
FROM conversations c
WHERE m.conversation_id = c.id;
