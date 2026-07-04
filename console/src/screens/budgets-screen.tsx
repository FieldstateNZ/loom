// BudgetsScreen — tenant cap cards + per-key consumption table + edit dialog.
import { useState } from "react";
import {
  Card, DataTable, BudgetBar, Badge, Button, IconButton, type Column,
} from "../components/index.ts";
import { BudgetEditDialog, type BudgetSubject } from "./budget-edit-dialog.tsx";
import type { LoomSnapshot, VirtualKey } from "../api/types.ts";

/** Props for {@link BudgetsScreen}. */
export interface BudgetsScreenProps {
  readonly data: LoomSnapshot;
  readonly role: "operator" | "tenant";
  readonly tenant: string;
}

/** Tenant cap cards + per-key consumption table, each editable via {@link BudgetEditDialog}. */
export function BudgetsScreen({ data, role, tenant }: BudgetsScreenProps) {
  const [editing, setEditing] = useState<BudgetSubject | null>(null);
  const tenants = role === "tenant" ? data.tenants.filter((t) => t.id === tenant) : data.tenants;
  const keys = (role === "tenant" ? data.keys.filter((k) => k.tenant === tenant) : data.keys).filter((k) => k.status !== "revoked");

  const columns: Column<VirtualKey>[] = [
    { key: "name", label: "Key", mono: true },
    ...(role === "operator" ? [{ key: "tenant", label: "Tenant", muted: true } as Column<VirtualKey>] : []),
    { key: "budget", label: "Consumption", width: "220px", render: (r) => r.cap ? <BudgetBar spent={r.budgetSpent} cap={r.cap} window={r.window} mode={r.mode} labels /> : <span style={{ color: "var(--fg-4)", font: "var(--w-reg) var(--fs-11)/1 var(--font-mono)" }}>no cap</span> },
    { key: "window", label: "Window", mono: true, muted: true, render: (r) => r.window || "—" },
    { key: "mode", label: "Over cap", render: (r) => <Badge tone={r.mode === "block" ? "danger" : "warn"} caps>{r.mode}</Badge> },
    { key: "rate", label: "Rate limit", align: "right", mono: true, muted: true, render: (r) => (r.rateRpm ?? 0) + " rpm" },
    { key: "edit", label: "", align: "right", width: "80px", render: (r) => <Button size="sm" variant="ghost" icon="pencil" onClick={() => setEditing(r)}>Edit</Button> },
  ];

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "12px" }}>
      <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(240px, 1fr))", gap: "10px" }}>
        {tenants.map((t) => (
          <Card key={t.id} eyebrow={t.name + " — " + t.window}
            actions={<IconButton icon="pencil" label={"Edit " + t.name + " budget"} size="sm"
              onClick={() => setEditing({ name: t.name.toLowerCase() + " (tenant)", cap: t.cap, window: t.window, mode: "block", rateRpm: 300 })} />}
            footer={<span>resets 1st of month · 00:00 UTC</span>}>
            <BudgetBar spent={t.spend30} cap={t.cap} window={t.window} labels />
          </Card>
        ))}
      </div>
      <Card eyebrow="Per-key budgets" flush footer={<span>Block refuses with 402 budget_exceeded; warn only flags the key. Tenant caps apply on top of key caps.</span>}>
        <DataTable rowKey="id" columns={columns} rows={keys} />
      </Card>
      <BudgetEditDialog subject={editing} onClose={() => setEditing(null)} />
    </div>
  );
}
