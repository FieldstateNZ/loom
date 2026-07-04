// KeysScreen — the virtual-key list, with search, the create-key flow and a
// danger-confirmed revoke.
import { useState } from "react";
import {
  Card, DataTable, Badge, BudgetBar, Button, IconButton, Input, Dialog, EmptyState,
  type Column, type BadgeTone,
} from "../components/index.ts";
import { Fmt } from "../lib/format.ts";
import { useLoom } from "../api/context.tsx";
import { CreateKeyDialog } from "./create-key-dialog.tsx";
import type { LoomSnapshot, VirtualKey } from "../api/types.ts";

/** Maps a key's status to the badge tone that signals it. */
const STATUS_TONE: Record<VirtualKey["status"], BadgeTone> = { active: "ok", blocked: "danger", revoked: "neutral" };

/** Props for {@link KeysScreen}. */
export interface KeysScreenProps {
  readonly data: LoomSnapshot;
  readonly role: "operator" | "tenant";
  readonly tenant: string;
}

/** Lists virtual keys (scoped to the tenant in tenant-admin mode) with create/revoke. */
export function KeysScreen({ data, role, tenant }: KeysScreenProps) {
  const client = useLoom();
  const formatMoney = Fmt.money;
  const [query, setQuery] = useState("");
  const [creating, setCreating] = useState(false);
  const [revoking, setRevoking] = useState<VirtualKey | null>(null);
  const [keys, setKeys] = useState<readonly VirtualKey[]>(data.keys);
  const scoped = role === "tenant" ? keys.filter((k) => k.tenant === tenant) : keys;
  const rows = scoped.filter((k) => k.name.includes(query.toLowerCase()));

  const columns: Column<VirtualKey>[] = [
    { key: "name", label: "Key", mono: true },
    ...(role === "operator" ? [{ key: "tenant", label: "Tenant", muted: true } as Column<VirtualKey>] : []),
    { key: "status", label: "Status", render: (r) => <Badge tone={STATUS_TONE[r.status]} caps icon={r.status === "blocked" ? "ban" : undefined}>{r.status}</Badge> },
    { key: "scopes", label: "Scopes", render: (r) => <span style={{ display: "flex", gap: "4px" }}>{r.scopes.map((s) => <Badge key={s}>{s}</Badge>)}</span> },
    { key: "budget", label: "Budget", width: "170px", render: (r) => r.status === "revoked" ? <span style={{ color: "var(--fg-4)" }}>—</span> : <BudgetBar spent={r.budgetSpent} cap={r.cap} window={r.window} mode={r.mode} labels /> },
    { key: "last", label: "Last used", muted: true, mono: true },
    { key: "spend7", label: "Spend — 7d", align: "right", mono: true, render: (r) => formatMoney(r.spend7) },
    { key: "actions", label: "", align: "right", width: "72px", render: (r) => (
      <span style={{ display: "inline-flex", gap: "2px" }}>
        <IconButton icon="pencil" label="Edit budget" size="sm" />
        <IconButton icon="trash-2" label="Revoke" variant="danger" size="sm" disabled={r.status === "revoked"} onClick={() => setRevoking(r)} />
      </span>
    ) },
  ];

  const revoke = async () => {
    if (!revoking) return;
    const res = await client.revokeKey(revoking.id);
    if (res.ok) setKeys(keys.map((k) => (k.id === res.value.id ? res.value : k)));
    setRevoking(null);
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "12px" }}>
      <div style={{ display: "flex", gap: "8px", justifyContent: "space-between" }}>
        <Input icon="search" size="sm" placeholder="Search keys…" value={query} onChange={setQuery} style={{ width: "260px" }} />
        <Button variant="primary" icon="plus" size="sm" onClick={() => setCreating(true)}>Create key</Button>
      </div>
      <Card flush>
        <DataTable
          rowKey="id"
          columns={columns}
          rows={rows}
          empty={<EmptyState icon="key" title="No keys match"
            hint="Issue a virtual key so a product can start talking to Loom."
            action={<Button variant="primary" icon="plus" onClick={() => setCreating(true)}>Create key</Button>} />}
        />
      </Card>
      <CreateKeyDialog open={creating} onClose={() => setCreating(false)} tenants={data.tenants}
        onCreated={(k) => setKeys([k, ...keys])} />
      <Dialog open={!!revoking} onClose={() => setRevoking(null)} danger title={revoking ? "Revoke " + revoking.name + "?" : ""}
        footer={<>
          <Button variant="ghost" onClick={() => setRevoking(null)}>Cancel</Button>
          <Button variant="danger" onClick={() => void revoke()}>Revoke key</Button>
        </>}>
        Requests using this key will fail immediately. This cannot be undone — issue a new key to restore access.
      </Dialog>
    </div>
  );
}
