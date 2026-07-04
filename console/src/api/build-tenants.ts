// Builds the operator-scope tenant list from the 30-day tenant usage rollup,
// enriching each row with tenant detail and its registered MCP servers. Several
// fields have no gateway read endpoint yet and degrade honestly (see
// HTTP_CLIENT_GAPS in http-client.ts).
import { asRecord, str } from "./json.ts";
import { totalRows, type UsageRow } from "./usage.ts";
import type { RequestFn } from "./http-request.ts";
import type { Tenant, McpServer } from "./models.ts";

/** The operator-scope collections derived from the tenant rollup. */
export interface TenantRollup {
  readonly tenants: readonly Tenant[];
  readonly mcpServers: readonly McpServer[];
}

/** Builds tenants + MCP servers from the tenant-grouped 30-day usage rollup. */
export async function buildTenants(
  request: RequestFn,
  adminToken: string,
  tenantRows: readonly UsageRow[],
): Promise<TenantRollup> {
  const totalCost = totalRows(tenantRows).cost;
  const mcpServers: McpServer[] = [];
  const tenants = await Promise.all(
    tenantRows.map((row) => buildTenant(request, adminToken, row, totalCost, mcpServers)),
  );
  return { tenants, mcpServers };
}

/** Maps one rollup row to a {@link Tenant}, appending its MCP servers to `out`. */
async function buildTenant(
  request: RequestFn,
  adminToken: string,
  row: UsageRow,
  totalCost: number,
  out: McpServer[],
): Promise<Tenant> {
  const id = row.group ?? "(unknown)";
  let name = id;
  let status: Tenant["status"] = "active";
  let mcpCount = 0;
  if (row.group) {
    const detailRes = await request(`/admin/tenants/${row.group}`, { token: adminToken, allow404: true });
    const detail = detailRes.ok ? asRecord(detailRes.value) : null;
    if (detail) {
      name = str(detail.name) ?? id;
      status = str(detail.status) === "active" ? "active" : "suspended";
    }
    mcpCount = await collectMcpServers(request, adminToken, row.group, name, out);
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
}

/** Appends a tenant's registered MCP servers to `out`; returns the count. */
async function collectMcpServers(
  request: RequestFn,
  adminToken: string,
  tenantId: string,
  tenantName: string,
  out: McpServer[],
): Promise<number> {
  const res = await request<unknown[]>(`/admin/tenants/${tenantId}/mcp-servers`, { token: adminToken });
  const servers = res.ok && Array.isArray(res.value) ? res.value : [];
  for (const raw of servers) {
    const s = asRecord(raw);
    if (!s) continue;
    out.push({
      id: str(s.id) ?? "",
      tenant: tenantName,
      name: str(s.name) ?? "",
      url: str(s.url) ?? "",
      // Registration-only: the gateway exposes no live health probe.
      status: "connected",
      last: str(s.updated_at) ?? "",
      tokenMeta: s.has_authorization ? "token set" : "no token",
    });
  }
  return servers.length;
}
