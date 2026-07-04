// Assembles the boot snapshot from the gateway's usage/admin endpoints. Two
// scopes contribute: a tenant virtual key drives the stat tiles and top-N bars;
// a root admin token drives the operator tenant list. Everything the gateway
// cannot serve yet is returned empty rather than faked (see HTTP_CLIENT_GAPS).
import { usageRollup, type RequestFn } from "./http-request.ts";
import { statsFrom, emptyStats } from "./usage.ts";
import { topModelBars, topKeyBars, usageByKeyRows } from "./usage-bars.ts";
import { buildTenants } from "./build-tenants.ts";
import type { LoomSnapshot } from "./snapshot.ts";
import type { GatewayStats, BarItem, UsageByKey } from "./metrics.ts";
import type { Tenant, McpServer } from "./models.ts";

/** The credentials that select which scopes of the snapshot can be populated. */
export interface BootstrapTokens {
  readonly adminToken?: string | undefined;
  readonly apiKey?: string | undefined;
}

/** Fetches and assembles the aggregate {@link LoomSnapshot} the console boots from. */
export async function buildBootstrap(
  request: RequestFn,
  { adminToken, apiKey }: BootstrapTokens,
): Promise<LoomSnapshot> {
  const now = new Date();
  const startToday = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate()));
  const startPrior = new Date(startToday.getTime() - 24 * 60 * 60 * 1000);
  const start30 = new Date(startToday.getTime() - 30 * 24 * 60 * 60 * 1000);
  const iso = (d: Date) => d.toISOString();

  let stats: GatewayStats = emptyStats();
  let topModels: BarItem[] = [];
  let topKeys: BarItem[] = [];
  let usageByKey: UsageByKey[] = [];
  let tenants: readonly Tenant[] = [];
  let mcpServers: readonly McpServer[] = [];

  // Tenant-scoped rollups (virtual key): stat tiles + top-N bars.
  if (apiKey) {
    const [modelRows, keyRows, todayRows, priorRows] = await Promise.all([
      usageRollup(request, "/v1/usage", apiKey, { group_by: "model", from: iso(start30) }),
      usageRollup(request, "/v1/usage", apiKey, { group_by: "key", from: iso(start30) }),
      usageRollup(request, "/v1/usage", apiKey, { group_by: "model", from: iso(startToday) }),
      usageRollup(request, "/v1/usage", apiKey, { group_by: "model", from: iso(startPrior), to: iso(startToday) }),
    ]);
    topModels = topModelBars(modelRows);
    topKeys = topKeyBars(keyRows);
    usageByKey = usageByKeyRows(keyRows);
    stats = statsFrom(todayRows, priorRows);
  }

  // Gateway-wide rollup + tenant detail (root token, operator scope).
  if (adminToken) {
    const tenantRows = await usageRollup(request, "/admin/usage", adminToken, {
      group_by: "tenant",
      from: iso(start30),
    });
    const built = await buildTenants(request, adminToken, tenantRows);
    tenants = built.tenants;
    mcpServers = built.mcpServers;

    // Operator-only deployments (no virtual key) still get real stat tiles.
    if (!apiKey) {
      const [todayRows, priorRows] = await Promise.all([
        usageRollup(request, "/admin/usage", adminToken, { group_by: "tenant", from: iso(startToday) }),
        usageRollup(request, "/admin/usage", adminToken, { group_by: "tenant", from: iso(startPrior), to: iso(startToday) }),
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
}

/** Formats a wall-clock "HH:MM:SS · Mon D" label for the snapshot header. */
function formatNow(d: Date): string {
  const time = d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false });
  const date = d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
  return `${time} · ${date}`;
}
