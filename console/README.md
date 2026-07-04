# Loom Console

The operator-facing web console for **Loom** — Fieldstate's open-source, multi-tenant
LLM gateway. It is the production React implementation of the Loom Console Design
System (a separate design handoff bundle, not vendored in this repo).

The gateway is the product; the console is how you see and steer it — spend, keys,
budgets, MCP servers, conversations, tenants, and provider credentials, role-scoped
to either a gateway **operator** or a single **tenant admin**.

## Stack

- **Vite + React 18 + TypeScript** (Fieldstate house stack).
- Design tokens are CSS custom properties (`src/styles/`), ported verbatim from the
  design system: warm-charcoal dark-first surfaces, the "dye rack" thread palette,
  IBM Plex Sans/Mono (self-hosted, OFL). Dark and light both ship.
- No UI dependencies beyond React — every component is hand-built against the tokens.

## Run

```bash
npm install
npm run dev        # dev server (HMR)
npm run build      # typecheck (tsc -b) + production build
npm run preview    # serve the production build
```

The console is deep-linkable — `?screen=keys&role=tenant&tenant=lucidbrain&theme=light`;
state syncs back to the URL so a refresh keeps your place.

## Layout

```
src/
  App.tsx              The console shell: role-scoped SideNav + TopBar + screen
                       router with tenant/provider drill-in. (= ConsoleScreen)
  main.tsx             Entry; loads token + component stylesheets, mounts <App>.
  api/
    types.ts           Domain model matching Loom's admin/usage REST shapes.
    client.ts          LoomClient interface — the one seam to a live gateway.
    mock.ts            createMockClient() — frozen demo data + latency; drop-in.
    context.tsx        <LoomProvider> / useLoom() so dialogs can mutate/probe.
  lib/format.ts        Numeric formatters (money, tokens, %, ms) — mono + tabular.
  components/          The design-system primitives, grouped by concern:
    core/  data/  forms/  feedback/  navigation/  transcript/
    index.ts           Barrel — screens import components from here.
  screens/             The eight v1 screens (one file each; Budgets/MCP split out).
  styles/              tokens.css (@imports) + components.css (global .lm-* rules).
```

## Going live against a real gateway

The console codes exclusively against the `LoomClient` interface (`src/api/client.ts`).
`createMockClient()` satisfies it from a frozen in-memory seed today. To point at a
running gateway, implement the same interface over HTTP (e.g. `createHttpClient(baseUrl)`
hitting Loom's OpenAPI endpoints) and pass it to `<LoomProvider client={…}>` in
`App.tsx`. Nothing else changes — the seam is one file.

## Screens

1. **Overview** — the money question: hero spend tile, token/request/stream tiles,
   spend-vs-prior chart, top models/keys, recent blocks & errors.
2. **Usage explorer** — filter chips, group-by, cost line, the cache read/write split
   (the caching ROI story), per-key pivot.
3. **Conversations** — searchable list + turn-by-turn transcript exercising every
   block type: text, thinking (collapsed), tool use/result (incl. error), web search
   with citations, code execution, cache markers, and the unknown-block raw-JSON
   fallback that never breaks the transcript.
4. **Keys** — dense table with inline budget bars; the create-key flow walks
   name → scopes → budget → the **shown-once** key moment; revoke via a danger confirm.
5. **Budgets & limits** — tenant cap cards + per-key consumption; edit dialog covers
   cap, window, block-vs-warn, rate limit.
6. **MCP servers** — register/edit with write-only tokens + connectivity check.
7. **Tenants** — operator list → tenant detail drill-in (that tenant's dashboard,
   keys, MCP servers, provider-credential override).
8. **Provider credentials** — providers as a collection (Anthropic is the first entry,
   not the only shape): list → per-provider credential, base URL, connectivity check,
   per-tenant overrides.

The sidenav context block toggles operator ↔ tenant-admin scope; the footer toggles
dark/light; the time-range control feeds the charts.
