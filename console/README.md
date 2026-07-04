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
Two implementations ship:

- **`createMockClient()`** (`src/api/mock.ts`) — a frozen in-memory seed. The
  design/dev default; used whenever no live base URL is configured.
- **`createHttpClient({ baseUrl, adminToken?, apiKey? })`** (`src/api/http.ts`) — the
  real client, hitting the gateway's OpenAPI endpoints (`clients/typescript/openapi.json`).

Selection is automatic (`src/api/context.tsx` → `resolveLoomClient()`): when a live base
URL resolves, the HTTP client is used; otherwise the mock. Nothing else changes.

### Configuration

Configure via Vite env (`.env.local`), a URL param, or `localStorage`, in precedence order:

| What        | URL param | localStorage       | Vite env                |
| ----------- | --------- | ------------------ | ----------------------- |
| Base URL    | `?api=…`  | `loom.baseUrl`     | `VITE_LOOM_BASE_URL`    |
| Admin token | —         | `loom.adminToken`  | `VITE_LOOM_ADMIN_TOKEN` |
| API key     | —         | `loom.apiKey`      | `VITE_LOOM_API_KEY`     |

```bash
# .env.local
VITE_LOOM_BASE_URL=https://gateway.example.com
VITE_LOOM_ADMIN_TOKEN=<root admin token>   # /admin surface (keys, tenants)
VITE_LOOM_API_KEY=loom_live_…              # tenant /v1 surface (transcripts, usage)
```

Tokens are deliberately **not** read from the URL (they would leak into history/logs) —
only `?api=` is, as a quick "point at that gateway" affordance. The gateway must allow the
console's origin (CORS) or be served same-origin / behind a proxy.

### Endpoint coverage (what's wired vs pending)

The gateway does not yet expose every collection/probe the console can show. The HTTP
client degrades honestly (empty arrays, nulls, typed "unsupported") rather than faking
data, and logs the shortfall on startup (`HTTP_CLIENT_GAPS` in `src/api/http.ts`).

**Fully wired**

- **Conversations** — `GET /v1/conversations/{id}` → the full turn-by-turn transcript,
  mapping loom-core `Message` / `ContentPart` into every console block type (text,
  thinking, tool use + result, server tools → web search / code exec, cache markers from
  usage, and the raw-JSON unknown fallback). Per-conversation totals come from
  `GET /v1/usage?group_by=conversation`.
- **Keys — create** — `POST /admin/tenants/{tenantId}/keys` (the shown-once secret), plus
  `PUT /admin/keys/{id}/budget` when a cap/window is set. (`input.tenant` must be the
  tenant **UUID**.)
- **Keys — revoke** — `DELETE /admin/keys/{id}`.
- **Overview / Usage stat tiles, Top models, Top keys, Usage-by-key** — computed from
  `GET /v1/usage` (`group_by=model|key`, day-windowed) and `GET /admin/usage?group_by=tenant`.
- **Tenants (partial)** — names/status/30-day spend/requests/MCP count from
  `GET /admin/usage?group_by=tenant` + `GET /admin/tenants/{id}` (+ its `mcp-servers`).

**Pending new gateway endpoints (degraded)**

- **Keys list** — no list-all-keys endpoint → `bootstrap().keys` is empty; `revokeKey`
  returns a minimal record (the gateway replies `204` with no body); `createKey` cannot
  set scopes (no scope-assignment endpoint).
- **Provider credentials** — no provider list/read endpoint → `providers` / `credOverrides`
  empty.
- **MCP servers** — listed per tenant, but `status` reflects *registration*, not a live
  `tools/list` probe.
- **Dashboard charts** — no time-series endpoint → `spendByHour` / `priorByHour` /
  `usageDaily` empty; **Events feed** — no events endpoint → empty.
- **Tenants** — per-tenant key count, budget cap/window and block counts have no read
  endpoint (`0` / `null` / `"monthly"`).
- **Connectivity checks** — no `/providers/{id}/check` or `/mcp/{id}/check` → the two
  `check*Connectivity` methods return a typed `{ ok: false, detail: "unsupported …" }`.

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
