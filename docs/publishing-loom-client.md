# Publishing `@fieldstate/loom-client`

## Decision

`@fieldstate/loom-client` (`clients/typescript`) stays an **unpublished
workspace artefact** for now. It is `"private": true` in `package.json` and
is consumed today only via a workspace/path dependency or a manually copied
tarball.

Publishing to any registry — private or public — requires a deliberate
go-ahead from a maintainer. This document exists so that when that
conversation happens, the target and the mechanics are already decided and
don't need to be re-litigated; it is not itself an instruction to publish,
and no publish workflow, registry token, or `publishConfig` has been added
alongside it.

## Recommended target: GitHub Packages (npm registry), private, Fieldstate-org-scoped

When we do publish, the recommendation is the **GitHub Packages npm
registry**, scoped to the `@fieldstate` org and kept **private**:

- **Keeps it internal.** The client wraps Loom's gateway API, which is
  itself internal infrastructure. A private registry avoids exposing the
  API surface (and its churn) to the public before we're ready to support
  it.
- **Reuses existing access control.** GitHub Packages authorizes against
  GitHub org membership/teams we already manage — no new secret store, no
  separate npm-org account, no additional credential to rotate or leak.
  CI publishing can use the ambient `GITHUB_TOKEN` (scoped to `packages:
  write` for the release job only) rather than a long-lived npm token.
- **Same toolchain.** It's still the standard npm registry protocol —
  consumers `npm install`, tooling doesn't change — just pointed at
  `npm.pkg.github.com` instead of `registry.npmjs.org`.

### Alternative considered: public npm under `@fieldstate`

Publishing publicly to npm (`@fieldstate/loom-client` on
`registry.npmjs.org`) was considered and **deferred**, because:

- It commits us to a **public, supported API surface** — semver
  guarantees, deprecation policy, issue triage from external consumers —
  before the client (and the gateway API it mirrors) has reached 1.0
  stability.
- The client's hand-authored models already document a known gap (rich
  `#[non_exhaustive]` server enums render as opaque `Object` in the OpenAPI
  spec today); shipping that publicly invites bug reports for a
  work-in-progress shape.
- There is no current external consumer. Nothing today needs public
  distribution, and GitHub Packages can always be graduated to public npm
  later without changing how the client is built or versioned.

If/when there's an external consumer or a stable public API commitment,
revisit this — the release mechanics below are largely reusable, only the
registry target and access policy change.

## Release flow, for when this is approved

None of the following is wired up yet. It's recorded here so the first
real publish is a matter of following a plan rather than inventing one.

### Version bump policy

The client version tracks the gateway's OpenAPI contract, not an
independent release cadence:

- **Patch** — internal refactors, doc/test changes, no change to
  `openapi.json` or generated types.
- **Minor** — additive, backward-compatible API changes (new endpoint, new
  optional field) reflected in a regenerated `openapi.json` /
  `src/generated.ts`.
- **Major** — breaking changes to the gateway API (removed/renamed field or
  endpoint, changed request/response shape) or a breaking change to the
  hand-authored models in `src/models/`.

In practice: whatever PR regenerates `openapi.json` and
`src/generated.ts` (see the `openapi-drift` CI job) should also bump
`clients/typescript/package.json`'s `version` per the rule above, and the
package version becomes the release tag.

### `publishConfig` (not yet added)

When approved, `clients/typescript/package.json` would gain:

```jsonc
// NOT YET ADDED — illustrative only
{
  "publishConfig": {
    "registry": "https://npm.pkg.github.com"
  }
}
```

and `"private": true` would be removed.

### Tag-triggered release workflow (sketch — not an active workflow)

A new, separate workflow (e.g. `.github/workflows/release-ts-client.yml`),
triggered on a version tag such as `ts-client-v*`, would look roughly
like:

```yaml
# SKETCH — not wired up. For illustration of the intended shape only.
name: release loom-client
on:
  push:
    tags: ["ts-client-v*"]
permissions:
  contents: read
  packages: write
jobs:
  publish:
    runs-on: ubuntu-latest
    defaults:
      run:
        working-directory: clients/typescript
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with:
          node-version: "22"
          registry-url: https://npm.pkg.github.com
          scope: "@fieldstate"
      - run: npm ci
      - run: npm run build
      - run: npm publish
        env:
          NODE_AUTH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
```

This would run the same `openapi-drift` guard (or depend on it) before
publishing, so a stale spec/client can never ship.

### Consumer setup

Because GitHub Packages requires the scope to be mapped to its registry,
consumers would add one line to their `.npmrc`:

```
@fieldstate:registry=https://npm.pkg.github.com
```

(plus a `GITHUB_TOKEN`/PAT with `read:packages` for `npm install` to
authenticate, per GitHub Packages' normal auth flow for private packages).

## Summary

| | Status |
| --- | --- |
| Published today? | No — private workspace artefact |
| Recommended target | GitHub Packages npm registry, private, `@fieldstate` org |
| Public npm | Considered, deferred pending 1.0 API stability |
| Publish workflow / token / `publishConfig` | Not added — pending maintainer sign-off |
