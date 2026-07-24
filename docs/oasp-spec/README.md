# Vendored OASP reference (read-only)

This directory is a read-only, offline copy of the canonical OASP (Open Agent
Session Protocol) adapter contract, vendored so fidelity between Loom's
`AgentProvider` trait (`crates/loom-provider/src/adapter.rs`) and the standard
can be checked without a network round-trip, and so reviewers can diff against
it directly.

- **Source repository:** [`github.com/oasp-dev/oasp-standard`](https://github.com/oasp-dev/oasp-standard)
- **Pinned commit:** `42f5e0c5f85827bbc6758c38282e9e692aa585cd`
- **Vendored on:** 2026-07-25

## Contents

- `adapter-types/` — the 8 canonical adapter TypeScript type files from
  `packages/conformance/src/adapter/` at the pinned commit.
- `spec/` — the normative adapter prose from `docs/spec/` at the pinned commit:
  `adapters.md`, `conversation-and-session.md`, `interactions.md`,
  `target-version-resolution.md`.

## Maintenance

These files are **not** part of Loom's build and are never imported by Rust
code — they exist purely as a fidelity reference. Do not hand-edit them.

When the standard advances past the pinned commit, re-sync by re-fetching the
same paths at the new commit SHA and updating the "Pinned commit" line above
(and this file's "Vendored on" date). Drift between Loom's contract and the
standard is tracked against the pinned SHA recorded here, not against the
standard's moving `main`.
