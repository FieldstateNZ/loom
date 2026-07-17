-- OASP Wave 3: pending tool-call model.
--
-- The server-side set of blocking tool calls a session is parked on — the
-- substrate for getPendingToolCalls, sendToolResult correlation, and drain.
-- A call is "pending" until its result is posted (resolved_at set).
--
-- tool_use_id is the correlation key: unique within a session, so sendToolResult
-- can match a result to exactly one call and reject an unknown or duplicate id.

CREATE TABLE pending_tool_calls (
    id             UUID PRIMARY KEY,
    session_id     UUID NOT NULL REFERENCES sessions (id) ON DELETE CASCADE,
    tool_use_id    TEXT NOT NULL,
    kind           TEXT NOT NULL,          -- 'custom' | 'builtin'
    name           TEXT NOT NULL,
    input          JSONB NOT NULL,
    mcp_server_url TEXT,                    -- true origin, when the adapter can vouch for it
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at    TIMESTAMPTZ,
    result         JSONB,
    is_error       BOOLEAN,
    CONSTRAINT uq_pending_tool_calls_session_tooluse UNIQUE (session_id, tool_use_id)
);

-- getPendingToolCalls read path: a session's still-blocking calls.
CREATE INDEX idx_pending_tool_calls_unresolved
    ON pending_tool_calls (session_id)
    WHERE resolved_at IS NULL;
