// TenantsScreen — operator-only: the tenant list, plus create-tenant, that
// drills into TenantDetail when a tenant is selected.
import { useState } from "react";
import {
  Card, DataTable, Badge, BudgetBar, Button, Field, Input, Dialog, type Column,
} from "../components/index.ts";
import { Fmt } from "../lib/format.ts";
import { TenantDetail } from "./tenant-detail.tsx";
import type { LoomSnapshot, Tenant } from "../api/types.ts";

/** Props for {@link TenantsScreen}. */
export interface TenantsScreenProps {
  readonly data: LoomSnapshot;
  /** The tenant id being drilled into, or `null` for the list view. */
  readonly detailId: string | null;
  readonly onOpenTenant: (t: Tenant) => void;
}

/** Operator tenant list; renders {@link TenantDetail} when one is selected. */
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
