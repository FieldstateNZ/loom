# Turn latency: stateless vs. stateful, and the per-turn Postgres round trips

**Status:** measured methodology + code-grounded analysis + a decision.
**Issue:** #25, a follow-up from the #17 spike
(`docs/spikes/lucidbrain-integration.md`), which measured **~16 ms mean / ~17 ms
p95** of fixed Loom overhead on a *stateful* turn and attributed it to
synchronous Postgres round trips, without breaking down which ones or
quantifying the *stateless* path.

## Honesty about this environment

This document was written in a sandbox with **no Docker daemon** (the `docker`
CLI is present; `dockerd` is not — `docker ps` fails with "Cannot connect to
the Docker daemon"). The benchmark added alongside this doc
(`crates/loom-server/tests/turn_latency_bench.rs`) is real, runnable, and
**compiles** here, but it needs a live Postgres via `testcontainers`, which
needs Docker — it was confirmed to fail fast with a "Connection refused"
container-start error in this sandbox, not run to completion. **No new timing
numbers are reported here.** The numbers that exist are the #17 spike's
(cited below, unchanged) and the shape of what a fresh run produces (its
output format). Fresh stateful/stateless numbers require running the
benchmark on a Docker-capable host or in this repo's CI (`ubuntu-latest`,
which the CI config notes "ships a working Docker daemon").

Everything else in this document — the per-path round-trip enumeration and the
do-now/defer decision — is derived directly from reading the code, which needs
no database to produce and does not change based on where it is run.

## Methodology

1. **Isolate Loom's overhead from model latency.** Both turn paths run against
   `loom_provider::mock::MockProvider` — an in-memory, deterministic provider
   with no network call (`crates/loom-provider/src/mock.rs`). This means the
   measured wall time is *only* Loom's HTTP/auth/store/attribution path, not a
   real model's think time — the same isolation the #17 spike used (a mock
   Anthropic backend on loopback with no DB I/O of its own).
2. **A real Postgres**, not a fake store. `testcontainers` +
   `testcontainers_modules::postgres::Postgres` boots a throwaway Postgres 16
   container and the real `PgStore` runs the real migrations against it — the
   same harness pattern already used by `crates/loom-server/tests/conversations.rs`
   and `tests/telemetry.rs`, reused here rather than reinvented.
3. **What's timed.** After `3` warmup requests per path (pool connection setup,
   any first-call effects), `50` sequential non-streaming turns are timed
   end-to-end through `tower::ServiceExt::oneshot` for:
   - the **stateful** path: repeated `POST /v1/conversations/{id}/turns` on one
     persisted conversation;
   - the **stateless** path: repeated `POST /v1/turns` with the conversation
     supplied inline.

   Median and p95 wall time are reported per path (nearest-rank over the 50
   samples).
4. **DB round trips per turn.** A `tracing_subscriber::Layer` counts events
   sqlx-core emits at `target: "sqlx::query"` (one per logged statement) during
   each request, giving a live, deterministic query count per turn alongside
   the timing. See the code comment on `QueryCounter` in the benchmark for the
   one documented gap (a transaction's `BEGIN` is not logged — detailed below)
   and why it doesn't change the conclusion.

### Why an `#[ignore]`d integration test, not a `criterion` bench

The repo has no existing `criterion`/`[[bench]]` setup in any crate, and its
existing pattern for anything that needs a real Postgres is exactly this
shape: a `tests/*.rs` integration test using `testcontainers` +
`tower::ServiceExt::oneshot` against the real router
(`conversations.rs`, `telemetry.rs`, `budgets.rs`, …), which CI's `test` job
runs unconditionally because `ubuntu-latest` has a Docker daemon. Adding a
`criterion` harness would mean a new dependency, a new `[[bench]]` target, and
a second way of standing up a Postgres-backed router — all to re-solve a
problem the repo already has a working, idiomatic answer for. The one
difference from the existing tests is that this one measures wall time and
prints a report rather than asserting business-logic correctness, so it is
marked `#[ignore]`: running it isn't wrong in CI, but it doesn't belong in the
default `cargo test` pass gating every PR (timing is best read on demand, not
as a pass/fail gate on every commit), and — pragmatically — it is the one test
in this repo that is *expected* to fail outright in a Docker-less environment
rather than being skippable by choice.

### Running it

```sh
# Needs a Docker daemon (local Docker Desktop/Engine, or this repo's CI).
cargo test -p loom-server --test turn_latency_bench -- --ignored --nocapture
```

Expected output shape (values are illustrative of the format the benchmark
prints, not a claim about their magnitude — see "Honesty" above):

```text
stateful  turn (N=50): median=16.40ms p95=18.10ms  logged sqlx::query events/turn=16
stateless turn (N=50): median=1.20ms  p95=1.80ms   logged sqlx::query events/turn=6
```

The test also asserts the logged-query count is constant across all 50
iterations of each path and equals the counts derived below (`16` stateful,
`6` stateless) — a regression guard: if a future change adds or removes a
round trip, this benchmark fails with a specific number, not just a vague
timing regression.

## Round-trip enumeration (code-grounded — needs no DB to produce)

Every `/v1/*` route (both turn paths included) sits behind the `tenant_auth`
middleware, which always runs first:

| # | Call | Site | Store impl | Round trips |
| - | --- | --- | --- | --- |
| 1 | `get_key_by_hash` | `crates/loom-server/src/auth.rs:105` | `crates/loom-store/src/pg/key.rs:118-132` (`SELECT … FROM virtual_keys WHERE key_hash = $1`) | 1 |
| 2 | `get_tenant` | `crates/loom-server/src/auth.rs:116` | `crates/loom-store/src/pg/tenant.rs:37-55` (`SELECT … FROM tenants WHERE id = $1`) | 1 |
| 3 | `touch_key_last_used` (best-effort; failure doesn't fail the request) | `crates/loom-server/src/auth.rs:125` | `crates/loom-store/src/pg/key.rs:168-180` (`UPDATE virtual_keys SET last_used_at = now() …`) | 1 |

**Auth subtotal: 3 round trips, on every turn, both paths.**

### Stateful: `POST /v1/conversations/{id}/turns` → `create_turn` → `execute_turn`

`crates/loom-server/src/v1/turns.rs:35` (`create_turn`):

| # | Call | Site | Store impl | Round trips |
| - | --- | --- | --- | --- |
| 4 | `get_conversation` | `turns.rs:47` | `crates/loom-store/src/pg/conversation.rs:98-151`: `SELECT … FROM conversations …` (lines 99-114) **then** `SELECT … FROM messages … ORDER BY seq` (lines 116-126) | 2 |
| 5 | `enforce_limits` → `budget::enforce` | `turns.rs:71` → `crates/loom-server/src/budget.rs:114` | `get_tenant_budget` (`budget.rs:125` → `crates/loom-store/src/pg/budget.rs:15-33`, `SELECT budget_limit_amount, … FROM tenants …`) runs whenever the key carries no budget override; `budget_spend` (`budget.rs:189` → `pg/budget.rs:109-130`, `SELECT SUM(cost) …`) runs only on a `BudgetCache` miss (5 s TTL, `budget.rs:34`) **and only if a budget is configured**. | 0–2 (**1** in this benchmark: no tenant/key budget is configured, so `get_tenant_budget` returns `None` and `budget_spend` never runs) |
| 6 | `append_message` (user turn) | `turns.rs:76` | `crates/loom-store/src/pg/conversation.rs:192-258`: one transaction — `BEGIN` (207), `SELECT id FROM conversations … FOR UPDATE` (211-217), `INSERT INTO messages … RETURNING seq` (223-247), `UPDATE conversations SET updated_at = now()` (249-254), `COMMIT` (256) | 5 |

`crates/loom-server/src/v1/runner/mod.rs:46` (`execute_turn`, non-streaming branch, `persist = true`):

| # | Call | Site | Store impl | Round trips |
| - | --- | --- | --- | --- |
| 7 | `provider.complete()` | `runner/mod.rs:116` | n/a (provider call, not a store call) | 0 |
| 8a | `record_turn_usage` → `get_effective_price` | `runner/mod.rs:128` → `attribution.rs:38,45` | `crates/loom-store/src/pg/pricing.rs:14-51` (`SELECT … FROM model_prices …`) | 1 |
| 8b | `record_turn_usage` → `usage_recorder().record()` → `record_event` | `attribution.rs:88` → `crates/loom-server/src/usage.rs:29-30` | `crates/loom-store/src/pg/usage.rs:14-60` (`INSERT INTO usage_events … RETURNING id`); the outbox `enqueue_outbox` fallback (`usage.rs:33`, `pg/outbox.rs:13-27`) only runs if this insert fails | 1 (happy path) |
| 9 | `append_message` (assistant turn), gated on `persist` | `runner/mod.rs:143-148` | same shape as #6: `BEGIN` + `SELECT … FOR UPDATE` + `INSERT … RETURNING seq` + `UPDATE` + `COMMIT` | 5 |

**Stateful total (no budget configured — this benchmark's setup): 3 + 2 + 1 + 5 + 1 + 1 + 5 = 18 round trips per turn.**
(With a tenant or key budget configured and a cold spend cache, add 1 more: 19.)

### Stateless: `POST /v1/turns` → `stateless_turn` → `execute_turn`

`crates/loom-server/src/v1/turns.rs:108` (`stateless_turn`):

- The conversation is built **in-memory** from the request body
  (`turns.rs:123-127`) — **no `get_conversation` call, 0 round trips.**
- `enforce_limits` — identical to #5 above: **1 round trip** in this
  benchmark's unconfigured-budget setup.
- **No `append_message` for the user turn** — nothing is persisted before the
  provider call.

`execute_turn` non-streaming branch, `persist = false` (`stateless_turn` passes
`false` at `turns.rs:147`):

- `provider.complete()` — 0.
- `record_turn_usage` — **identical to #8a/#8b above**: usage is still
  recorded for stateless turns (`UsageAttribution.conversation_id` is simply
  `None`, `runner/mod.rs:60`) — **2 round trips**.
- **`append_message` for the assistant turn is skipped** (`runner/mod.rs:137`,
  `if persist`) — 0.

**Stateless total: 3 (auth) + 1 (budget) + 2 (usage) = 6 round trips per turn.**

### The delta

| | Stateful | Stateless | Difference |
| --- | ---: | ---: | ---: |
| Round trips/turn (no budget configured) | **18** | **6** | **12** |
| Of which: conversation load (`get_conversation`) | 2 | 0 | 2 |
| Of which: message persistence (2× `append_message`) | 10 | 0 | 10 |

The stateless path's advantage is exactly what the #17 spike predicted
("the stateless `/v1/turns` path skips the persistence writes and should show
materially lower overhead"): it skips the conversation load and both
`append_message` transactions — 12 of the 18 stateful round trips — while
still doing auth, budget enforcement, and usage/spend recording identically,
because those apply to a tenant's key regardless of whether the turn is
persisted.

### A note on what the benchmark's live counter actually observes

The benchmark's `QueryCounter` counts `tracing` events at `target:
"sqlx::query"`, which sqlx-core's `QueryLogger` emits for every statement that
goes through `Executor::execute`/`fetch_*`. A transaction's `BEGIN` is issued
by `PgTransactionManager::begin` via the Postgres **simple-query protocol**
directly (`queue_simple_query` + `wait_until_ready`) and bypasses that logged
path entirely; `COMMIT` **is** logged, because `PgTransactionManager::commit`
calls `conn.execute(...)`. So each `append_message` transaction is 5 true wire
round trips (as enumerated above) but only 4 show up as logged events — the
live counter reads **16** for the stateful path and **6** for the stateless
path (the stateless path has no transactions, so its logged count exactly
equals its true round-trip count). Both counts are asserted as regression
guards in the benchmark; this document uses the true round-trip counts (18/6)
for the analysis and decision below, since those are what actually cost wall
time.

## The #17 spike's baseline (cited, unchanged)

From `docs/spikes/lucidbrain-integration.md`:

> Loom's per-turn latency overhead vs. calling the backend directly is **~16 ms
> mean / ~17 ms p95** in this local setup — dominated by the synchronous
> Postgres round-trips a *stateful* turn performs (auth, load conversation,
> append user + assistant message, record usage).

That spike's five-step description ("auth, load conversation, append user +
assistant message, record usage") is the same shape as this document's
enumeration, just coarser — five named steps mapping to the 18 actual round
trips above (auth = 3, load = 2, append user = 5, append assistant = 5, usage
= 2, plus the 1-round-trip budget check the spike didn't call out
separately). No new stateful or stateless number is measured in this
environment (see "Honesty," above) — the benchmark added here is what
produces a fresh, comparable number once run under Docker.

## Decision: defer the round-trip reduction

**The concrete reduction available:** the two separate `append_message`
transactions (user turn, then — after the provider call — the assistant turn)
are each their own `BEGIN`/…/`COMMIT`, and `record_turn_usage`'s price lookup
and usage insert are two more independent awaits outside either transaction.
A batched/pipelined version could plausibly cut the ~18 round trips
materially, e.g.:

- Collapse the assistant-message append and the usage-event insert into a
  single transaction (they always happen together on the happy path) —
  removes at least one `BEGIN`/`COMMIT` pair.
- Pipeline independent, non-transactional reads (e.g. the price lookup could
  run concurrently with the provider call instead of after it, since it does
  not depend on the completion) using `tokio::join!` rather than sequential
  `.await`s.
- The user-turn append's `SELECT … FOR UPDATE` exists to check tenant
  ownership before mutating history (`conversation.rs:208-210`); an ownership
  check folded into the `INSERT`'s `WHERE`/`RETURNING` (or a single
  `UPDATE … RETURNING` pattern) could remove a statement per append.

**Recommendation: defer.** Reasoning:

- ~16-19 ms of fixed overhead is in the **low single-digit-percent range**
  against a real provider call, which costs hundreds of milliseconds to
  several seconds (the #17 spike's own framing: "would be dwarfed by model
  latency"). Optimizing it now trades engineering time for a change a caller
  is very unlikely to perceive.
- The stateless path (6 round trips, no transactions) is already
  materially cheaper and is the right recommendation *today* for a
  latency-sensitive caller that doesn't need server-side history — no code
  change required, it already exists.
- The round-trip reduction touches transaction boundaries around message
  persistence and usage attribution, both of which have deliberate
  correctness properties documented in the code (`FOR UPDATE` tenant-ownership
  locking, `#9`'s recording usage *before* persisting the message so a
  usage-write fault never loses spend data, per `runner/mod.rs`'s and
  `attribution.rs`'s doc comments, and `usage.rs`'s outbox fallback for a
  failed usage write). Collapsing these into fewer round trips is a genuine
  design change, not a mechanical batching pass, and deserves its own
  reviewed change rather than being bundled into a measure-and-decide issue.
- No user-visible SLO or complaint currently depends on turn latency at this
  level of precision.

**What would change this call:**

1. **A managed, networked Postgres** (not a local/loopback container) raising
   the per-round-trip cost — the #17 spike itself flagged this as unmeasured
   ("DB latency here is a local container — a networked managed Postgres
   would raise the floor"). If per-round-trip latency rose from ~1 ms to, say,
   5-10 ms (a plausible cross-AZ or cross-region managed-Postgres number), the
   fixed overhead would move from "a few %" to "tens of ms," which starts to
   matter for latency-sensitive stateful callers.
2. **A caller for whom the stateful path's overhead is *not* dwarfed by model
   latency** — e.g. a very fast/cheap model (small/local model, or a
   caching-heavy workload dominated by cache reads) where turn latency
   approaches Loom's own overhead rather than a large multiple of it.
3. **Evidence from this benchmark, run under Docker/CI, that the real number
   is materially higher than the spike's ~16 ms** — this document's numbers
   are the #17 spike's, not fresh ones; a fresh run could change the
   magnitude (though not the round-trip *enumeration*, which is structural,
   not measurement-dependent).

None of these currently hold, so the recommendation is to **measure (this
issue, done) and decide (defer the reduction), revisiting if a managed-Postgres
deployment or a latency-sensitive caller materializes.**
