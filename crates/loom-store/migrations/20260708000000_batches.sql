-- Anthropic Message Batches: asynchronous bulk processing at the discounted
-- batch tier.
--
-- Design notes
-- ------------
-- * A batch job is a set of stateless turn requests (the inline
--   {provider, model, system?, messages, options?} shape of POST /v1/turns).
--   Items are stored verbatim as JSONB so submission is lossless.
-- * RESULTS RETENTION: results are STORED (fetched-through-then-persisted), not
--   re-fetched from the provider on every read. When a batch ends the poll
--   worker retrieves the provider's JSONL results once and writes each item's
--   result into `batch_items.result`; GET /v1/batches/{id}/results then streams
--   straight from the store. This keeps reads cheap and independent of the
--   provider's results-URL retention window, at the cost of storing the result
--   payloads (bounded by the batch size the caller submitted).
-- * BATCH PRICING: rather than duplicate every price row for a batch tier, a
--   single `batch_multiplier` column on model_prices carries the discount
--   factor (1.0 = none, 0.5 = Anthropic's 50%-off batch tier). It is applied to
--   token charges when recording a batch item's usage; `usage_events.is_batch`
--   marks those events so rollups can distinguish batch from interactive spend.

-- Pricing: the batch-tier token-charge multiplier ---------------------------
ALTER TABLE model_prices
    ADD COLUMN batch_multiplier NUMERIC NOT NULL DEFAULT 1.0;

-- Anthropic's Message Batches API bills at 50% of the standard token rates.
UPDATE model_prices SET batch_multiplier = 0.5 WHERE provider = 'anthropic';

-- Usage: distinguish batch-tier events from interactive ones ----------------
ALTER TABLE usage_events
    ADD COLUMN is_batch BOOLEAN NOT NULL DEFAULT false;

-- Batch jobs ----------------------------------------------------------------
CREATE TABLE batch_jobs (
    id                UUID PRIMARY KEY,
    tenant_id         UUID NOT NULL REFERENCES tenants (id) ON DELETE CASCADE,
    virtual_key_id    UUID REFERENCES virtual_keys (id) ON DELETE SET NULL,
    provider          TEXT NOT NULL,
    status            TEXT NOT NULL DEFAULT 'created',
    provider_batch_id TEXT,
    results_url       TEXT,
    total_items       INTEGER NOT NULL DEFAULT 0,
    processing_count  INTEGER NOT NULL DEFAULT 0,
    succeeded_count   INTEGER NOT NULL DEFAULT 0,
    errored_count     INTEGER NOT NULL DEFAULT 0,
    canceled_count    INTEGER NOT NULL DEFAULT 0,
    expired_count     INTEGER NOT NULL DEFAULT 0,
    error             TEXT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    ended_at          TIMESTAMPTZ
);

CREATE INDEX idx_batch_jobs_tenant ON batch_jobs (tenant_id);

-- The poll worker scans jobs that are still advancing, oldest-first. A partial
-- index keeps that scan cheap as ended jobs accumulate.
CREATE INDEX idx_batch_jobs_active
    ON batch_jobs (created_at)
    WHERE status <> 'ended';

-- Batch items ---------------------------------------------------------------
-- `request` holds the verbatim inline turn; `result` holds the resolved
-- per-item outcome (assistant message on success, provider error on failure).
CREATE TABLE batch_items (
    id         UUID PRIMARY KEY,
    batch_id   UUID NOT NULL REFERENCES batch_jobs (id) ON DELETE CASCADE,
    tenant_id  UUID NOT NULL REFERENCES tenants (id) ON DELETE CASCADE,
    custom_id  TEXT NOT NULL,
    seq        INTEGER NOT NULL,
    model      TEXT NOT NULL,
    status     TEXT NOT NULL DEFAULT 'pending',
    request    JSONB NOT NULL,
    result     JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT uq_batch_items_custom UNIQUE (batch_id, custom_id)
);

-- Results read path and worker submission: ordered by (batch_id, seq).
CREATE INDEX idx_batch_items_batch_seq ON batch_items (batch_id, seq);
