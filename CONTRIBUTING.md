# Contributing to Loom

Thanks for helping build Loom. This is a young project moving issue-by-issue;
please open or comment on an issue before large changes so we can keep the
architecture coherent.

## Ground rules

1. **Provider fidelity is sacred.** `loom-core` must not assume an OpenAI shape,
   and provider libraries must never silently drop a native capability. If Loom
   doesn't model something yet, carry it through `ProviderExtension` rather than
   discarding it.
2. **Every store query is tenant-scoped.** Cross-tenant reads are a security
   bug, not a convenience gap.
3. **No prompt/completion content in telemetry by default.**

## Local setup

```bash
rustup toolchain install stable        # rust-toolchain.toml pins stable + components
cargo build --workspace
docker compose up -d db                 # Postgres 16 for integration tests
cargo test --workspace
```

Integration tests use [testcontainers](https://docs.rs/testcontainers) and need
a running Docker daemon; they spin up their own PostgreSQL. The `loom-store`
tests can instead target an already-running server (a fresh database is
created per test): set `LOOM_TEST_DATABASE_URL=postgres://user:pass@host/postgres`,
and optionally `LOOM_TEST_EXPECT_PG_MAJOR=18` to fail unless that server is the
expected major version. CI uses this to prove migrations on PostgreSQL 18.

## Before you push

CI runs exactly these — run them locally first:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

- Keep public types documented (`#![warn(missing_docs)]` is on for library crates).
- Mark enums/structs `#[non_exhaustive]` where future variants are likely.
- Provider changes need fixture-based tests; no live API calls in CI (gate any
  live test behind an env flag, e.g. `LOOM_ANTHROPIC_LIVE=1`).

## Commit / PR style

- Small, reviewable PRs mapped to an issue.
- Conventional-ish commit subjects (`feat(core): …`, `fix(anthropic): …`) are
  appreciated but not enforced.

## Licence of contributions

By contributing you agree your work is licensed under the project's
[Apache-2.0](./LICENSE) licence, and that you have the right to submit it under
that licence (inbound = outbound, per Apache-2.0 section 5).
