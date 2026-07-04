// TenantsScreen — operator-only: tenant list + drill-in tenant detail
// (that tenant's dashboard + keys + MCP servers + credential override).
import { useState } from "react";
import {
  Card, StatTile, DataTable, Badge, BudgetBar, LineChart, StatusDot, Button,
  SecretInput, Field, Input, Dialog, type Column, type BadgeTone,
} from "../components/index.ts";
import { Fmt } from "../lib/format.ts";
import type { LoomSnapshot, Tenant, VirtualKey, McpServer } from "../api/types.ts";

const STATUS_TONE: Record<VirtualKey["status"], BadgeTone> = { active: "ok", blocked: "danger", revoked: "neutral" };

function TenantDetail({ data, t }: { data: LoomSnapshot; t: Tenant }) {
  const formatMoney = Fmt.money, formatTokens = Fmt.tokens;
  const keys = data.keys.filter((k) => k.tenant === t.id);
  const mcp = data.mcpServers.filter((m) => m.tenant === t.id);
  const cred = data.credOverrides.find((c) => c.tenant === t.id);
  const u = data.usageDaily;
  const spendSeries = u.cost.map((v) => v * t.share);

  const keyColumns: Column<VirtualKey>[] = [
    { key: "name", label: "Key", mono: true },
    { key: "status", label: "Status", render: (r) => <Badge tone={STATUS_TONE[r.status]} caps icon={r.status === "blocked" ? "ban" : undefined}>{r.status}</Badge> },
    { key: "scopes", label: "Scopes", render: (r) => <span style={{ display: "flex", gap: "4px" }}>{r.scopes.map((s) => <Badge key={s}>{s}</Badge>)}</span> },
    { key: "budget", label: "Budget", width: "170px", render: (r) => r.status === "revoked" ? <span style={{ color: "var(--fg-4)" }}>—</span> : <BudgetBar spent={r.budgetSpent} cap={r.cap} window={r.window} mode={r.mode} labels /> },
    { key: "last", label: "Last used", muted: true, mono: true },
    { key: "spend7", label: "Spend — 7d", align: "right", mono: true, render: (r) => formatMoney(r.spend7) },
  ];
  const mcpColumns: Column<McpServer>[] = [
    { key: "name", label: "Server", mono: true },
    { key: "url", label: "URL", mono: true, muted: true },
    { key: "status", label: "Auth", render: (r) => <StatusDot tone={r.status === "connected" ? "ok" : "danger"} label={r.status} /> },
    { key: "last", label: "Last used", mono: true, muted: true },
  ];

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "12px" }}>
      <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(180px, 1fr))", gap: "10px" }}>
        <StatTile hero label="Spend — 30d" labelRight={t.window} value={formatMoney(t.spend30)}
          sub={<BudgetBar spent={t.spend30} cap={t.cap} window={t.window} labels />} style={{ gridColumn: "span 2" }} />
        <StatTile label="Requests — 30d" value={formatTokens(t.requests30)} spark={spendSeries} sparkColor="var(--series-4)" />
        <StatTile label="Keys" value={String(keys.filter((k) => k.status !== "revoked").length)} sub={keys.some((k) => k.status === "blocked") ? <StatusDot tone="danger" label="1 blocked" /> : <StatusDot tone="ok" label="all healthy" />} />
        <StatTile label="Budget blocks — 30d" value={String(t.blocks30)} sub={t.blocks30 ? "most recent 14:32 today" : "none this window"} />
      </div>
      <Card eyebrow={"Spend — 7d · " + t.name.toLowerCase()}>
        <LineChart area height={150}
          series={[{ name: "spend", color: "var(--series-1)", data: spendSeries }]}
          yFormat={(v) => formatMoney(v, { compact: true })}
          xLabels={[u.labels[0] ?? "", "", "", u.labels[3] ?? "", "", "", u.labels[6] ?? ""]} />
      </Card>
      <Card eyebrow="Keys" flush>
        <DataTable rowKey="id" columns={keyColumns} rows={keys} />
      </Card>
      <div style={{ display: "grid", gridTemplateColumns: "1.4fr 1fr", gap: "10px", alignItems: "start" }}>
        <Card eyebrow="MCP servers" flush>
          <DataTable
            rowKey="id"
            columns={mcpColumns}
            rows={mcp}
            empty={<div style={{ padding: "20px", font: "var(--w-reg) var(--fs-12)/1.5 var(--font-sans)", color: "var(--fg-3)" }}>No MCP servers registered for this tenant.</div>}
          />
        </Card>
        <Card eyebrow="Provider credential"
          actions={cred && cred.set ? <Badge tone="accent" caps>Override</Badge> : <Badge caps>Inherits gateway</Badge>}>
          {cred && cred.set ? (
            <Field hint="This tenant uses its own Anthropic key instead of the gateway default.">
              <SecretInput isSet meta={cred.meta} />
            </Field>
          ) : (
            <div style={{ display: "flex", flexDirection: "column", gap: "10px" }}>
              <p style={{ margin: 0, font: "var(--w-reg) var(--fs-12)/1.5 var(--font-sans)", color: "var(--fg-3)" }}>
                Requests use the gateway's Anthropic credential. Set an override to bill this tenant to its own provider account.
              </p>
              <Button size="sm" icon="shield" style={{ alignSelf: "flex-start" }}>Set override</Button>
            </div>
          )}
        </Card>
      </div>
    </div>
  );
}

export interface TenantsScreenProps {
  data: LoomSnapshot;
  detailId: string | null;
  onOpenTenant: (t: Tenant) => void;
}

export function TenantsScreen({ data, detailId, onOpenTenant }: TenantsScreenProps) {
  const formatMoney = Fmt.money;
  const [query, setQuery] = useState("");
  const [creating, setCreating] = useState(false);
  const [name, setName] = useState("");
  const detail = detailId ? data.tenants.find((t) => t.id === detailId) : undefined;
  if (detail) return <TenantDetail data={data} t={detail} />;
  const rows = data.tenants.filter((t) => t.name.toLowerCase().includes(query.toLowerCase()));

  const columns: Column<Tenant>[] = [
    { key: "name", label: "Tenant" },
    { key: "id", label: "ID", mono: true, muted: true },
    { key: "status", label: "Status", render: (r) => <Badge tone="ok" caps>{r.status}</Badge> },
    { key: "keys", label: "Keys", align: "right", mono: true },
    { key: "mcp", label: "MCP", align: "right", mono: true },
    { key: "blocks", label: "Blocks — 30d", align: "right", mono: true, render: (r) => r.blocks30 ? <span style={{ color: "var(--danger)", fontWeight: "var(--w-med)" }}>{r.blocks30}</span> : <span style={{ color: "var(--fg-4)" }}>0</span> },
    { key: "budget", label: "Budget — monthly", width: "190px", render: (r) => <BudgetBar spent={r.spend30} cap={r.cap} window={r.window} labels /> },
    { key: "spend30", label: "Spend — 30d", align: "right", mono: true, render: (r) => formatMoney(r.spend30) },
  ];

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "12px" }}>
      <div style={{ display: "flex", gap: "8px", justifyContent: "space-between" }}>
        <Input icon="search" size="sm" placeholder="Search tenants…" value={query} onChange={setQuery} style={{ width: "260px" }} />
        <Button variant="primary" icon="plus" size="sm" onClick={() => setCreating(true)}>Create tenant</Button>
      </div>
      <Card flush footer={<span>Tenant admins see only their own tenant — scope is enforced by the gateway, not the UI.</span>}>
        <DataTable rowKey="id" onRowClick={(r) => onOpenTenant(r)} columns={columns} rows={rows} />
      </Card>
      <Dialog open={creating} onClose={() => { setCreating(false); setName(""); }} title="Create tenant" width={440}
        footer={<>
          <Button variant="ghost" onClick={() => { setCreating(false); setName(""); }}>Cancel</Button>
          <Button variant="primary" disabled={!name.trim()} onClick={() => { setCreating(false); setName(""); }}>Create tenant</Button>
        </>}>
        <div style={{ display: "flex", flexDirection: "column", gap: "14px" }}>
          <Field label="Name" hint="The product or team this tenant isolates — keys, budgets and MCP servers scope to it." required>
            <Input autoFocus value={name} onChange={setName} placeholder="LucidBrain" />
          </Field>
          <Field label="Tenant ID">
            <Input mono readOnly value={(name || "tenant").toLowerCase().replace(/[^a-z0-9]+/g, "-")} />
          </Field>
        </div>
      </Dialog>
    </div>
  );
}
