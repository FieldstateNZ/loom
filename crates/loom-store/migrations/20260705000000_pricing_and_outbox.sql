-- Spend tracking: versioned pricing, a usage outbox, and the columns the
-- rollup query API groups by.
--
-- The usage_events table already exists (initial schema). This migration adds
-- the pricing model, the failure-mode outbox, and seed prices.

-- Model prices -------------------------------------------------------------
-- VERSIONED, append-only: a price change is a NEW row with a later
-- effective_from, never an in-place update. The effective price for an event
-- is the latest row (provider, model) with effective_from <= the event time.
-- Cost is computed at write time from the effective price and stored on
-- usage_events.cost; the raw usage is preserved so cost can be recomputed if a
-- price was wrong.
CREATE TABLE model_prices (
    id                   UUID PRIMARY KEY,
    provider             TEXT NOT NULL,
    model                TEXT NOT NULL,
    input_per_mtok       NUMERIC NOT NULL,
    output_per_mtok      NUMERIC NOT NULL,
    cache_write_per_mtok NUMERIC NOT NULL,
    cache_read_per_mtok  NUMERIC NOT NULL,
    server_tool_prices   JSONB NOT NULL DEFAULT '{}'::jsonb,
    currency             TEXT NOT NULL DEFAULT 'USD',
    effective_from       TIMESTAMPTZ NOT NULL,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- At most one price per (provider, model) version instant. A correction to
    -- an existing version updates in place; a genuine price change uses a new
    -- effective_from.
    CONSTRAINT uq_model_prices_version UNIQUE (provider, model, effective_from)
);

-- Effective-price lookup: latest effective_from <= at, for a (provider, model).
CREATE INDEX idx_model_prices_lookup
    ON model_prices (provider, model, effective_from DESC);

-- Usage outbox -------------------------------------------------------------
-- FAILURE MODE: a usage write failure must NOT fail the user's turn. When the
-- primary usage_events insert fails, the event is parked here (pending) and a
-- drain/retry path reprocesses it. The full NewUsageEvent is stored as JSONB
-- so the replay is byte-for-byte identical.
CREATE TABLE usage_outbox (
    id           UUID PRIMARY KEY,
    payload      JSONB NOT NULL,
    status       TEXT NOT NULL DEFAULT 'pending',
    attempts     INTEGER NOT NULL DEFAULT 0,
    last_error   TEXT,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    processed_at TIMESTAMPTZ
);

-- Drain path scans pending rows oldest-first.
CREATE INDEX idx_usage_outbox_pending
    ON usage_outbox (created_at)
    WHERE status = 'pending';

-- Rollup grouping indexes on usage_events.
CREATE INDEX idx_usage_events_key ON usage_events (tenant_id, virtual_key_id);
CREATE INDEX idx_usage_events_model ON usage_events (tenant_id, model);
CREATE INDEX idx_usage_events_conversation ON usage_events (tenant_id, conversation_id);

-- Seed: current Anthropic prices (USD per million tokens) --------------------
--
-- ‼️ VERIFY these against https://www.anthropic.com/pricing before relying on
-- them for billing. Sourced from the bundled `claude-api` skill's model table
-- (cached 2026-06-24), which supersedes the placeholder figures in the issue.
-- The versioned schema makes a correction a trivial new row with a later
-- effective_from — never edit these in place for a genuine price change.
--
--   Model                          input / output  ($/MTok)
--   claude-opus-4-8                5    / 25
--   claude-sonnet-5                3    / 15   (intro 2/10 through 2026-08-31)
--   claude-haiku-4-5-20251001      1    / 5
--
-- Cache: cache_write = 1.25x input (5-min TTL), cache_read = 0.10x input.
-- Web search server tool: $10 per 1000 requests = 0.01 / request, keyed by the
-- Anthropic usage field name `web_search_requests`.
INSERT INTO model_prices (
    id, provider, model,
    input_per_mtok, output_per_mtok, cache_write_per_mtok, cache_read_per_mtok,
    server_tool_prices, currency, effective_from
) VALUES
    (
        gen_random_uuid(), 'anthropic', 'claude-opus-4-8',
        5, 25, 6.25, 0.50,
        '{"web_search_requests": 0.01}'::jsonb, 'USD', TIMESTAMPTZ '2026-01-01 00:00:00+00'
    ),
    (
        gen_random_uuid(), 'anthropic', 'claude-sonnet-5',
        3, 15, 3.75, 0.30,
        '{"web_search_requests": 0.01}'::jsonb, 'USD', TIMESTAMPTZ '2026-01-01 00:00:00+00'
    ),
    (
        gen_random_uuid(), 'anthropic', 'claude-haiku-4-5-20251001',
        1, 5, 1.25, 0.10,
        '{"web_search_requests": 0.01}'::jsonb, 'USD', TIMESTAMPTZ '2026-01-01 00:00:00+00'
    );
