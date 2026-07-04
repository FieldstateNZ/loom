// createHttpClient — a LoomClient backed by the live Loom gateway's REST API.
//
// The console codes exclusively against the LoomClient interface (client.ts).
// createMockClient (mock.ts) satisfies it from a frozen seed for design/dev;
// this module satisfies the SAME interface against a running gateway, returning
// the SAME shapes the mock returns. Selection lives in context.tsx: a live base
// URL (env / ?api= / localStorage) selects this client, otherwise the mock.
//
// ── Gateway endpoints this client uses ─────────────────────────────────────
//   Virtual-key auth (Authorization: Bearer loom_…, opts.apiKey):
//     GET  /v1/whoami                          resolved tenant identity
//     GET  /v1/conversations/{id}              → getTranscript (mapped below)
//     GET  /v1/usage?group_by=model|key        → topModels / topKeys / usageByKey / stats
//   Root-token auth (Authorization: Bearer <root>, opts.adminToken):
//     POST   /admin/tenants/{tenantId}/keys    → createKey (the shown-once secret)
//     PUT    /admin/keys/{id}/budget           → createKey budget (cap/window/mode)
//     DELETE /admin/keys/{id}                  → revokeKey
//     GET    /admin/tenants/{id}               → tenant name/status (bootstrap)
//     GET    /admin/tenants/{id}/mcp-servers   → mcpServers (bootstrap)
//     GET    /admin/usage?group_by=tenant      → tenants rollup / stats (bootstrap)
//
// ── Console features that DEGRADE (no gateway endpoint yet) ─────────────────
// The gateway exposes no list/collection endpoints for several console screens,
// and no connectivity probes. Rather than fabricate data, this client returns
// honest empty/partial results and records the shortfall in HTTP_CLIENT_GAPS
// (logged once on construction). Pending new gateway endpoints:
//   • Keys screen        — no list-all-keys endpoint → bootstrap().keys = []
//                          revokeKey() cannot read the full record back (204 only)
//                          createKey() cannot set scopes (no scope endpoint)
//   • Provider creds     — no provider list / read endpoint → providers = [],
//                          credOverrides = []
//   • Tenants            — partial: names/status/spend/requests/mcp are real;
//                          per-tenant key count, budget cap/window and block
//                          counts have no read endpoint (0 / null / "monthly")
//   • MCP servers        — listed per tenant (operator scope), but health status
//                          is NOT probed: `status` reflects *registration*, not a
//                          live tools/list check
//   • Dashboard series   — no time-bucketed endpoint → spendByHour / priorByHour /
//                          usageDaily are empty; stat tiles are computed from
//                          day-window usage rollups
//   • Events feed        — no events endpoint → events = []
//   • Connectivity probes— no /providers/{id}/check or /mcp/{id}/check endpoint →
//                          check*Connectivity return a typed "unsupported" result
//   • Transcript.key     — the conversation payload does not carry the owning
//                          virtual key's display name (falls back to metadata.key)

import type { LoomClient } from "./client.ts";
import type {
  LoomSnapshot,
  Transcript,
  TranscriptTurn,
  TranscriptBlock,
  ToolUseBlock,
  WebSearchBlock,
  WebSearchResult,
  CodeExecBlock,
  TurnUsage,
  VirtualKey,
  CreateKeyInput,
  ConnectivityResult,
  Tenant,
  McpServer,
  BarItem,
  UsageByKey,
  GatewayStats,
} from "./types.ts";

/** Configuration for {@link createHttpClient}. */
export interface HttpClientOptions {
  /** The gateway's base URL, e.g. `https://gateway.example.com` (no trailing slash needed). */
  baseUrl: string;
  /** The root admin token, for the `/admin` surface (key/tenant provisioning). */
  adminToken?: string | undefined;
  /** A tenant virtual key (`loom_…`), for the tenant-scoped `/v1` surface. */
  apiKey?: string | undefined;
}

/**
 * The console capabilities the live gateway cannot satisfy yet. Surfaced so the
 * gap is honest and discoverable rather than silently faked. Logged once when a
 * client is constructed; also mirrored in this file's header and console/README.
 */
export const HTTP_CLIENT_GAPS: readonly string[] = [
  "bootstrap().keys is empty — no list-all-keys endpoint on the gateway",
  "bootstrap().providers / credOverrides are empty — no provider list/read endpoint",
  "bootstrap().tenants omits key-count, budget cap/window and block counts — no read endpoints",
  "bootstrap().mcpServers status is registration-only — no live connectivity probe",
  "bootstrap().spendByHour / priorByHour / usageDaily are empty — no time-series endpoint",
  "bootstrap().events is empty — no gateway events endpoint",
  "revokeKey() returns a minimal record — the gateway replies 204 with no body and has no read-back",
  "createKey() cannot set scopes — the gateway has no scope-assignment endpoint",
  "checkProviderConnectivity / checkMcpConnectivity are unsupported — no probe endpoints",
];

let gapsAnnounced = false;

// ── Small utilities ────────────────────────────────────────────────────────

/** Coerces a JSON number-or-numeric-string (rust_decimal serializes as a string) to a number. */
function num(v: unknown): number {
  if (typeof v === "number") return Number.isFinite(v) ? v : 0;
  if (typeof v === "string") {
    const n = Number.parseFloat(v);
    return Number.isFinite(n) ? n : 0;
  }
  return 0;
}

/** Narrows an unknown JSON value to a record, or null. */
function asRecord(v: unknown): Record<string, unknown> | null {
  return v && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, unknown>) : null;
}

function str(v: unknown): string | undefined {
  return typeof v === "string" ? v : undefined;
}

/** An HTTP error carrying the response status for callers that branch on it. */
class HttpError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.name = "HttpError";
    this.status = status;
  }
}

// ── Usage rollup row (both /v1/usage and /admin/usage share this shape) ─────
interface UsageRow {
  group: string | null;
  event_count: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  cost: number;
}

function toUsageRow(raw: unknown): UsageRow {
  const r = asRecord(raw) ?? {};
  return {
    group: str(r.group) ?? null,
    event_count: num(r.event_count),
    input_tokens: num(r.input_tokens),
    output_tokens: num(r.output_tokens),
    cache_read_tokens: num(r.cache_read_tokens),
    cache_write_tokens: num(r.cache_write_tokens),
    cost: num(r.cost),
  };
}

interface RollupTotals {
  cost: number;
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  events: number;
}

function totalRows(rows: UsageRow[]): RollupTotals {
  return rows.reduce<RollupTotals>(
    (a, r) => ({
      cost: a.cost + r.cost,
      input: a.input + r.input_tokens,
      output: a.output + r.output_tokens,
      cacheRead: a.cacheRead + r.cache_read_tokens,
      cacheWrite: a.cacheWrite + r.cache_write_tokens,
      events: a.events + r.event_count,
    }),
    { cost: 0, input: 0, output: 0, cacheRead: 0, cacheWrite: 0, events: 0 },
  );
}

function pctDelta(current: number, prior: number): number {
  if (prior <= 0) return 0;
  return Math.round(((current - prior) / prior) * 100);
}

const SERIES_COLORS = ["var(--series-1)", "var(--series-2)", "var(--series-3)", "var(--series-4)"];

// ── Client ──────────────────────────────────────────────────────────────────

export function createHttpClient(opts: HttpClientOptions): LoomClient {
  const baseUrl = opts.baseUrl.replace(/\/+$/, "");
  const { adminToken, apiKey } = opts;

  if (!gapsAnnounced) {
    gapsAnnounced = true;
    // eslint-disable-next-line no-console
    console.info(
      "[loom] live HTTP client active against %s. Known coverage gaps:\n  - %s",
      baseUrl,
      HTTP_CLIENT_GAPS.join("\n  - "),
    );
  }

  /** Fetches `path`, returning parsed JSON. Returns null on 404 when allowed. */
  async function request<T = unknown>(
    path: string,
    init: { method?: string; token?: string; body?: unknown; allow404?: boolean } = {},
  ): Promise<T | null> {
    const headers: Record<string, string> = { accept: "application/json" };
    if (init.token) headers.authorization = `Bearer ${init.token}`;
    if (init.body !== undefined) headers["content-type"] = "application/json";

    const res = await fetch(`${baseUrl}${path}`, {
      method: init.method ?? "GET",
      headers,
      ...(init.body !== undefined ? { body: JSON.stringify(init.body) } : {}),
    });

    if (res.status === 404 && init.allow404) return null;
    if (res.status === 204) return null;
    if (!res.ok) {
      let detail = `${res.status} ${res.statusText}`;
      try {
        const body = await res.json();
        const rec = asRecord(body);
        const msg = rec && (str(rec.message) ?? str((asRecord(rec.error) ?? {}).message) ?? str(rec.error));
        if (msg) detail = msg;
      } catch {
        /* non-JSON error body */
      }
      throw new HttpError(res.status, detail);
    }
    return (await res.json()) as T;
  }

  /** GET a usage rollup, coercing rows; returns [] on any failure. */
  async function usageRollup(
    path: string,
    token: string | undefined,
    params: Record<string, string>,
  ): Promise<UsageRow[]> {
    if (!token) return [];
    const qs = new URLSearchParams(params).toString();
    try {
      const body = await request<Record<string, unknown>>(`${path}?${qs}`, { token });
      const rows = body && Array.isArray(body.rows) ? (body.rows as unknown[]) : [];
      return rows.map(toUsageRow);
    } catch {
      return [];
    }
  }

  return {
    async bootstrap(): Promise<LoomSnapshot> {
      const now = new Date();
      const startToday = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate()));
      const startPrior = new Date(startToday.getTime() - 24 * 60 * 60 * 1000);
      const start30 = new Date(startToday.getTime() - 30 * 24 * 60 * 60 * 1000);
      const iso = (d: Date) => d.toISOString();

      let stats = emptyStats();
      let topModels: BarItem[] = [];
      let topKeys: BarItem[] = [];
      let usageByKey: UsageByKey[] = [];
      let tenants: Tenant[] = [];
      const mcpServers: McpServer[] = [];

      // Tenant-scoped rollups (virtual key).
      if (apiKey) {
        const [modelRows, keyRows, todayRows, priorRows] = await Promise.all([
          usageRollup("/v1/usage", apiKey, { group_by: "model", from: iso(start30) }),
          usageRollup("/v1/usage", apiKey, { group_by: "key", from: iso(start30) }),
          usageRollup("/v1/usage", apiKey, { group_by: "model", from: iso(startToday) }),
          usageRollup("/v1/usage", apiKey, { group_by: "model", from: iso(startPrior), to: iso(startToday) }),
        ]);

        topModels = modelRows
          .slice()
          .sort((a, b) => b.cost - a.cost)
          .slice(0, 6)
          .map((r, i) => ({
            label: r.group ?? "(unknown)",
            value: r.cost,
            display: `$${r.cost.toFixed(2)}`,
            color: SERIES_COLORS[i % SERIES_COLORS.length] ?? "var(--series-1)",
          }));

        topKeys = keyRows
          .slice()
          .sort((a, b) => b.cost - a.cost)
          .slice(0, 6)
          .map((r) => ({ label: r.group ?? "(unknown)", value: r.cost, display: `$${r.cost.toFixed(2)}` }));

        usageByKey = keyRows.map((r) => ({
          key: r.group ?? "(unknown)",
          requests: r.event_count,
          input: r.input_tokens,
          output: r.output_tokens,
          cacheRead: r.cache_read_tokens,
          cacheWrite: r.cache_write_tokens,
          cost: r.cost,
        }));

        stats = statsFrom(todayRows, priorRows);
      }

      // Gateway-wide rollups + tenant detail (root token, operator scope).
      if (adminToken) {
        const tenantRows = await usageRollup("/admin/usage", adminToken, {
          group_by: "tenant",
          from: iso(start30),
        });
        const totalCost = totalRows(tenantRows).cost;

        const built = await Promise.all(
          tenantRows.map(async (row): Promise<Tenant> => {
            const id = row.group ?? "(unknown)";
            let name = id;
            let status: Tenant["status"] = "active";
            let mcpCount = 0;
            if (row.group) {
              const detail = asRecord(
                await request(`/admin/tenants/${row.group}`, { token: adminToken, allow404: true }).catch(() => null),
              );
              if (detail) {
                name = str(detail.name) ?? id;
                status = str(detail.status) === "active" ? "active" : "suspended";
              }
              const servers = await request<unknown[]>(`/admin/tenants/${row.group}/mcp-servers`, {
                token: adminToken,
              }).catch(() => null);
              if (Array.isArray(servers)) {
                mcpCount = servers.length;
                for (const raw of servers) {
                  const s = asRecord(raw);
                  if (!s) continue;
                  mcpServers.push({
                    id: str(s.id) ?? "",
                    tenant: name,
                    name: str(s.name) ?? "",
                    url: str(s.url) ?? "",
                    // Registration-only: the gateway exposes no live health probe.
                    status: "connected",
                    last: str(s.updated_at) ?? "",
                    tokenMeta: s.has_authorization ? "token set" : "no token",
                  });
                }
              }
            }
            return {
              id,
              name,
              status,
              keys: 0, // no per-tenant key-count endpoint
              mcp: mcpCount,
              spend30: row.cost,
              cap: null, // no tenant-budget read endpoint
              window: "monthly",
              share: totalCost > 0 ? row.cost / totalCost : 0,
              requests30: row.event_count,
              blocks30: 0, // no block-count endpoint
            };
          }),
        );
        tenants = built;

        // Operator-only deployments (no virtual key) still get real stat tiles.
        if (!apiKey) {
          const [todayRows, priorRows] = await Promise.all([
            usageRollup("/admin/usage", adminToken, { group_by: "tenant", from: iso(startToday) }),
            usageRollup("/admin/usage", adminToken, {
              group_by: "tenant",
              from: iso(startPrior),
              to: iso(startToday),
            }),
          ]);
          stats = statsFrom(todayRows, priorRows);
        }
      }

      return {
        now: formatNow(now),
        keys: [], // no list-all-keys endpoint
        tenants,
        credOverrides: [], // no credential read endpoint
        providers: [], // no provider list endpoint
        stats,
        spendByHour: [], // no time-series endpoint
        priorByHour: [],
        usageDaily: { labels: [], cost: [], input: [], output: [], cacheRead: [], cacheWrite: [] },
        topModels,
        topKeys,
        events: [], // no events endpoint
        usageByKey,
        mcpServers,
        conversations: [], // no list-all-conversations endpoint
      };
    },

    async getTranscript(conversationId: string): Promise<Transcript | null> {
      if (!apiKey) {
        // Without a virtual key the tenant-scoped endpoint is unreachable.
        return null;
      }
      const conv = asRecord(
        await request(`/v1/conversations/${conversationId}`, { token: apiKey, allow404: true }),
      );
      if (!conv) return null;

      const binding = asRecord(conv.binding) ?? {};
      const model = str(binding.model) ?? "";
      const metadata = asRecord(conv.metadata);
      const key = (metadata && str(metadata.key)) ?? "";
      const messages = Array.isArray(conv.messages) ? (conv.messages as unknown[]) : [];

      // Correlates tool-use / server-tool-use blocks (by id) with the tool_result /
      // server_tool_result parts that arrive in a later message, so the console
      // renders the call and its result merged into one block.
      const pending = new Map<string, ToolUseBlock | WebSearchBlock | CodeExecBlock>();
      const turns: TranscriptTurn[] = messages.map((raw) =>
        mapMessage(asRecord(raw) ?? {}, model, pending),
      );

      const totals = await transcriptTotals(conversationId, turns);
      return { id: str(conv.id) ?? conversationId, key, model, totals, turns };
    },

    async createKey(input: CreateKeyInput): Promise<{ key: VirtualKey; secret: string }> {
      if (!adminToken) throw new Error("createKey requires an admin token (VITE_LOOM_ADMIN_TOKEN).");

      const created = asRecord(
        await request(`/admin/tenants/${input.tenant}/keys`, {
          method: "POST",
          token: adminToken,
          body: { name: input.name, env: "live" },
        }),
      );
      if (!created) throw new Error("createKey: empty response from gateway");

      const id = str(created.id) ?? "";
      const secret = str(created.key) ?? "";

      // Apply the requested budget if one was specified (separate admin call).
      if (id && input.cap !== null && input.window !== null) {
        await request(`/admin/keys/${id}/budget`, {
          method: "PUT",
          token: adminToken,
          body: { limit_amount: input.cap, window: input.window, action: input.mode },
        }).catch((err) => {
          // eslint-disable-next-line no-console
          console.warn("[loom] key created but budget not applied:", err);
        });
      }

      const key: VirtualKey = {
        id,
        name: str(created.name) ?? input.name,
        tenant: input.tenant,
        status: "active",
        // Echo the requested scopes: the gateway has no scope-assignment endpoint,
        // so these reflect intent, not persisted state (see HTTP_CLIENT_GAPS).
        scopes: input.scopes,
        budgetSpent: 0,
        cap: input.cap,
        window: input.window,
        mode: input.mode,
        last: "just now",
        spend7: 0,
      };
      return { key, secret };
    },

    async revokeKey(id: string): Promise<VirtualKey> {
      if (!adminToken) throw new Error("revokeKey requires an admin token (VITE_LOOM_ADMIN_TOKEN).");
      await request(`/admin/keys/${id}`, { method: "DELETE", token: adminToken });
      // The gateway replies 204 with no body and exposes no key read-back, so we
      // can only report the revoked status against the id (see HTTP_CLIENT_GAPS).
      return {
        id,
        name: id,
        tenant: "",
        status: "revoked",
        scopes: [],
        budgetSpent: 0,
        cap: null,
        window: null,
        mode: "block",
        last: "just now",
        spend7: 0,
      };
    },

    async checkProviderConnectivity(_providerId: string): Promise<ConnectivityResult> {
      return {
        ok: false,
        detail: "unsupported — the gateway exposes no provider connectivity probe endpoint yet",
      };
    },

    async checkMcpConnectivity(_serverId: string): Promise<ConnectivityResult> {
      return {
        ok: false,
        detail: "unsupported — the gateway exposes no MCP connectivity probe endpoint yet",
      };
    },
  };

  // ── getTranscript helpers (close over `request`/`apiKey`) ─────────────────

  /**
   * Real per-conversation totals from the usage rollup (cost + tokens), falling
   * back to summing per-turn token usage (cost 0) when the rollup is unavailable.
   */
  async function transcriptTotals(
    conversationId: string,
    turns: TranscriptTurn[],
  ): Promise<Transcript["totals"]> {
    const rows = await usageRollup("/v1/usage", apiKey, { group_by: "conversation" });
    const row = rows.find((r) => r.group === conversationId);
    if (row) {
      return {
        cost: row.cost,
        inTok: row.input_tokens,
        outTok: row.output_tokens,
        cacheRead: row.cache_read_tokens,
        cacheWrite: row.cache_write_tokens,
      };
    }
    return turns.reduce(
      (a, t) => ({
        cost: a.cost,
        inTok: a.inTok + (t.usage?.inTok ?? 0),
        outTok: a.outTok + (t.usage?.outTok ?? 0),
        cacheRead: a.cacheRead + (t.usage?.cacheRead ?? 0),
        cacheWrite: a.cacheWrite + (t.usage?.cacheWrite ?? 0),
      }),
      { cost: 0, inTok: 0, outTok: 0, cacheRead: 0, cacheWrite: 0 },
    );
  }
}

// ── Message / ContentPart → console transcript mapping ───────────────────────

function mapMessage(
  msg: Record<string, unknown>,
  convModel: string,
  pending: Map<string, ToolUseBlock | WebSearchBlock | CodeExecBlock>,
): TranscriptTurn {
  const rawRole = str(msg.role);
  const role: TranscriptTurn["role"] = rawRole === "user" ? "user" : rawRole === "assistant" ? "assistant" : "system";

  const usage = mapUsage(asRecord(msg.usage));
  const blocks: TranscriptBlock[] = [];

  // Surface the turn's cache read/write as markers, from real usage counts.
  if (usage?.cacheWrite) blocks.push({ type: "cache", kind: "write", tokens: usage.cacheWrite });
  if (usage?.cacheRead) blocks.push({ type: "cache", kind: "read", tokens: usage.cacheRead });

  const parts = Array.isArray(msg.content) ? (msg.content as unknown[]) : [];
  for (const raw of parts) {
    const part = asRecord(raw);
    if (!part) continue;
    mapPart(part, blocks, pending);
  }

  const turn: TranscriptTurn = { role, blocks };
  if (role === "assistant" && convModel) turn.model = convModel;
  if (usage) turn.usage = usage;
  return turn;
}

function mapUsage(u: Record<string, unknown> | null): TurnUsage | undefined {
  if (!u) return undefined;
  const usage: TurnUsage = {};
  if (u.input_tokens != null) usage.inTok = num(u.input_tokens);
  if (u.output_tokens != null) usage.outTok = num(u.output_tokens);
  if (u.cache_read_tokens != null) usage.cacheRead = num(u.cache_read_tokens);
  if (u.cache_write_tokens != null) usage.cacheWrite = num(u.cache_write_tokens);
  return Object.keys(usage).length > 0 ? usage : undefined;
}

const WEB_SEARCH_NAMES = /web_search/i;
const CODE_EXEC_NAMES = /code_(execution|interpreter)|bash_code/i;

/** Maps one loom ContentPart into 0..1 transcript blocks (mutating `blocks`). */
function mapPart(
  part: Record<string, unknown>,
  blocks: TranscriptBlock[],
  pending: Map<string, ToolUseBlock | WebSearchBlock | CodeExecBlock>,
): void {
  const type = str(part.type) ?? "unknown";

  switch (type) {
    case "text": {
      blocks.push({ type: "text", text: str(part.text) ?? "" });
      return;
    }
    case "thinking": {
      blocks.push({ type: "thinking", text: str(part.thinking) ?? "" });
      return;
    }
    case "redacted_thinking": {
      blocks.push({ type: "thinking", text: "[redacted reasoning]" });
      return;
    }
    case "tool_use": {
      const block: ToolUseBlock = { type: "tool_use", name: str(part.name) ?? "tool", input: part.input };
      const id = str(part.id);
      if (id) pending.set(id, block);
      blocks.push(block);
      return;
    }
    case "tool_result": {
      const id = str(part.tool_use_id);
      const target = id ? pending.get(id) : undefined;
      if (target && target.type === "tool_use") {
        target.result = part.content;
        if (part.is_error != null) target.isError = Boolean(part.is_error);
      } else {
        blocks.push({
          type: "tool_use",
          name: "tool_result",
          result: part.content,
          isError: part.is_error != null ? Boolean(part.is_error) : undefined,
        });
      }
      return;
    }
    case "server_tool_use": {
      const name = str(part.name) ?? "server_tool";
      const id = str(part.id);
      const input = asRecord(part.input);
      if (WEB_SEARCH_NAMES.test(name)) {
        const block: WebSearchBlock = { type: "web_search", query: (input && str(input.query)) ?? "", results: [] };
        if (id) pending.set(id, block);
        blocks.push(block);
      } else if (CODE_EXEC_NAMES.test(name)) {
        const block: CodeExecBlock = { type: "code_exec" };
        if (input) {
          const code = str(input.code);
          const lang = str(input.language);
          if (code) block.code = code;
          if (lang) block.lang = lang;
        }
        if (id) pending.set(id, block);
        blocks.push(block);
      } else {
        const block: ToolUseBlock = { type: "tool_use", name, via: "server", input: part.input };
        if (id) pending.set(id, block);
        blocks.push(block);
      }
      return;
    }
    case "server_tool_result": {
      const id = str(part.tool_use_id);
      const target = id ? pending.get(id) : undefined;
      if (target?.type === "web_search") {
        target.results = parseWebSearchResults(part.content);
      } else if (target?.type === "code_exec") {
        applyCodeExecResult(target, part.content);
      } else if (target?.type === "tool_use") {
        target.result = part.content;
      } else {
        blocks.push({ type: "unknown", blockType: "server_tool_result", data: part });
      }
      return;
    }
    case "provider_extension": {
      blocks.push({ type: "unknown", blockType: str(part.kind) ?? "provider_extension", data: part.payload ?? part });
      return;
    }
    default: {
      // image, document, and any future/unmodelled part — preserved verbatim.
      blocks.push({ type: "unknown", blockType: type, data: part });
    }
  }
}

/** Best-effort extraction of web-search results from a provider-native payload. */
function parseWebSearchResults(content: unknown): WebSearchResult[] {
  const items = Array.isArray(content)
    ? content
    : Array.isArray(asRecord(content)?.content)
      ? (asRecord(content)!.content as unknown[])
      : [];
  const out: WebSearchResult[] = [];
  for (const raw of items) {
    const r = asRecord(raw);
    if (!r) continue;
    const url = str(r.url);
    const title = str(r.title);
    if (!url && !title) continue;
    const result: WebSearchResult = { title: title ?? url ?? "", url: url ?? "" };
    // Only surface a genuine snippet — never the provider's opaque encrypted blob.
    const snippet = str(r.snippet) ?? str(r.description);
    if (snippet) result.snippet = snippet;
    out.push(result);
  }
  return out;
}

/** Best-effort extraction of stdout/stderr/exit code from a code-execution payload. */
function applyCodeExecResult(block: CodeExecBlock, content: unknown): void {
  const r = asRecord(content) ?? asRecord(asRecord(content)?.content);
  if (!r) return;
  const stdout = str(r.stdout);
  const stderr = str(r.stderr);
  if (stdout != null) block.stdout = stdout;
  if (stderr != null) block.stderr = stderr;
  if (r.return_code != null) block.exitCode = num(r.return_code);
  else if (r.exit_code != null) block.exitCode = num(r.exit_code);
}

// ── stats helpers ────────────────────────────────────────────────────────────

function emptyStats(): GatewayStats {
  return {
    spendToday: 0,
    spendPrior: 0,
    spendDelta: 0,
    tokensIn: 0,
    tokensInDelta: 0,
    tokensOut: 0,
    tokensOutDelta: 0,
    requests: 0,
    requestsDelta: 0,
    streams: 0,
    cacheReadToday: 0,
    cacheWriteToday: 0,
    cacheSavedToday: 0,
    cacheHitRate: 0,
  };
}

function statsFrom(todayRows: UsageRow[], priorRows: UsageRow[]): GatewayStats {
  const t = totalRows(todayRows);
  const p = totalRows(priorRows);
  const cacheDenom = t.cacheRead + t.input;
  return {
    spendToday: t.cost,
    spendPrior: p.cost,
    spendDelta: pctDelta(t.cost, p.cost),
    tokensIn: t.input,
    tokensInDelta: pctDelta(t.input, p.input),
    tokensOut: t.output,
    tokensOutDelta: pctDelta(t.output, p.output),
    requests: t.events,
    requestsDelta: pctDelta(t.events, p.events),
    streams: 0, // no active-stream count endpoint
    cacheReadToday: t.cacheRead,
    cacheWriteToday: t.cacheWrite,
    cacheSavedToday: 0, // needs pricing the console cannot see
    cacheHitRate: cacheDenom > 0 ? t.cacheRead / cacheDenom : 0,
  };
}

function formatNow(d: Date): string {
  const time = d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false });
  const date = d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
  return `${time} · ${date}`;
}
