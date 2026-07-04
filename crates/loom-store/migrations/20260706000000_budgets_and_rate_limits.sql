-- Budgets and rate limits (#10).
--
-- Budgets are attachable at the TENANT and the KEY level; a key-level budget
-- overrides the tenant default. virtual_keys already carries budget columns
-- (from #6); this migration adds the equivalent tenant-level budget columns and
-- the per-key rate-limit columns.
--
-- Spend for the current window is computed from usage_events.cost (the #9 store)
-- at enforcement time; no spend is denormalised here.

-- Tenant-level budget --------------------------------------------------------
-- A tenant either has a complete budget (all three columns set) or none. The
-- window is one of daily|weekly|monthly|total; the action is block|warn.
ALTER TABLE tenants
    ADD COLUMN budget_limit_amount NUMERIC,
    ADD COLUMN budget_window       TEXT,
    ADD COLUMN budget_action       TEXT;

-- Per-key rate limits --------------------------------------------------------
-- Enforced by an in-process token bucket (single-instance for v1; distributed
-- limiting across replicas is deferred — see the README). NULL means unlimited
-- on that dimension.
ALTER TABLE virtual_keys
    ADD COLUMN rate_limit_requests_per_min BIGINT,
    ADD COLUMN rate_limit_tokens_per_min   BIGINT;
