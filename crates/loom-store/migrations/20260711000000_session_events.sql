-- OASP Wave 2: normalised session event log.
--
-- The closed, ordered event stream for a session — the durable backing for
-- `stream` (SSE) and cursor-paginated `listSessionEvents`, which doubles as the
-- audit source. Each event carries a per-session monotonic `seq`; its external
-- order-comparable id is the zero-padded `seq`, so lexicographic ordering
-- matches emission order (the adapter contract's event-ordering invariant).

CREATE TABLE session_events (
    session_id UUID NOT NULL REFERENCES sessions (id) ON DELETE CASCADE,
    seq        BIGINT NOT NULL,
    kind       JSONB NOT NULL,
    at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (session_id, seq)
);

CREATE INDEX idx_session_events_session_seq ON session_events (session_id, seq);
