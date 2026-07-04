// KeysScreen — list + the create-key flow, including the shown-once moment.
import { useState } from "react";
import {
  Card, DataTable, Badge, BudgetBar, Button, IconButton, Input, Dialog, EmptyState,
  Field, Select, Switch, RevealOnce, type Column, type BadgeTone,
} from "../components/index.ts";
import { Fmt } from "../lib/format.ts";
import { useLoom } from "../api/context.tsx";
import type { LoomSnapshot, VirtualKey, Tenant, CreateKeyInput, BudgetWindow } from "../api/types.ts";

const STATUS_TONE: Record<VirtualKey["status"], BadgeTone> = { active: "ok", blocked: "danger", revoked: "neutral" };

interface CreateKeyDialogProps {
  open: boolean;
  onClose: () => void;
  onCreated: (key: VirtualKey) => void;
  tenants: Tenant[];
}

function CreateKeyDialog({ open, onClose, onCreated, tenants }: CreateKeyDialogProps) {
  const client = useLoom();
  const [step, setStep] = useState(0);
  const [name, setName] = useState("");
  const [tenant, setTenant] = useState(tenants[0] ? tenants[0].id : "");
  const [scopes, setScopes] = useState<Record<string, boolean>>({ messages: true, streaming: true, mcp: false });
  const [capOn, setCapOn] = useState(true);
  const [cap, setCap] = useState("50");
  const [win, setWin] = useState<BudgetWindow>("daily");
  const [block, setBlock] = useState(true);
  const [busy, setBusy] = useState(false);
  const [secret, setSecret] = useState<string | null>(null);
  const [created, setCreated] = useState<VirtualKey | null>(null);

  const reset = () => {
    setStep(0); setName(""); setScopes({ messages: true, streaming: true, mcp: false });
    setCapOn(true); setCap("50"); setWin("daily"); setBlock(true);
    setBusy(false); setSecret(null); setCreated(null);
  };
  const close = () => { reset(); onClose(); };

  const doCreate = async () => {
    const input: CreateKeyInput = {
      name, tenant,
      scopes: Object.keys(scopes).filter((k) => scopes[k]),
      cap: capOn ? Number(cap) : null,
      window: capOn ? win : null,
      mode: block ? "block" : "warn",
    };
    setBusy(true);
    const res = await client.createKey(input);
    setSecret(res.secret);
    setCreated(res.key);
    setBusy(false);
    setStep(2);
  };

  const scopeRow = (id: string, label: string, hint: string) => (
    <Switch key={id} checked={scopes[id] ?? false} onChange={(v) => setScopes({ ...scopes, [id]: v })}
      label={<span>{label} <span style={{ color: "var(--fg-3)" }}>— {hint}</span></span>} />
  );

  if (step === 2 && created && secret) {
    return (
      <Dialog open={open} title="Key created" icon="circle-check" width={520}
        footer={<Button variant="primary" onClick={() => { onCreated(created); close(); }}>Done — I've stored it</Button>}>
        <div style={{ display: "flex", flexDirection: "column", gap: "12px" }}>
          <div style={{ display: "flex", gap: "6px", alignItems: "center", flexWrap: "wrap" }}>
            <Badge>{created.name || "unnamed"}</Badge>
            <Badge>{created.tenant}</Badge>
            {created.scopes.map((k) => <Badge key={k} tone="info">{k}</Badge>)}
            {capOn ? <Badge tone="accent">${cap} {win} · {block ? "block" : "warn"}</Badge> : <Badge>no budget</Badge>}
          </div>
          <RevealOnce secret={secret} />
        </div>
      </Dialog>
    );
  }
  return (
    <Dialog open={open} onClose={close} title={step === 0 ? "Create key" : "Set a budget"} width={480}
      footer={<>
        <Button variant="ghost" onClick={step === 0 ? close : () => setStep(0)}>{step === 0 ? "Cancel" : "Back"}</Button>
        <Button variant="primary" loading={busy} disabled={step === 0 && !name.trim()}
          onClick={() => (step === 0 ? setStep(1) : void doCreate())}>{step === 0 ? "Continue" : "Create key"}</Button>
      </>}>
      {step === 0 ? (
        <div style={{ display: "flex", flexDirection: "column", gap: "14px" }}>
          <Field label="Key name" hint="Shown in usage tables — name it product/env." required>
            <Input mono autoFocus value={name} onChange={setName} placeholder="lucidbrain-prod" />
          </Field>
          <Field label="Tenant">
            <Select options={tenants.map((t) => ({ value: t.id, label: t.name }))} value={tenant} onChange={setTenant} />
          </Field>
          <Field label="Scopes">
            <div style={{ display: "flex", flexDirection: "column", gap: "9px", paddingTop: "2px" }}>
              {scopeRow("messages", "messages", "send conversations")}
              {scopeRow("streaming", "streaming", "SSE responses")}
              {scopeRow("mcp", "mcp", "reference registered MCP servers")}
            </div>
          </Field>
        </div>
      ) : (
        <div style={{ display: "flex", flexDirection: "column", gap: "14px" }}>
          <Switch checked={capOn} onChange={setCapOn} label="Cap spend on this key" />
          {capOn ? (
            <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: "12px" }}>
              <Field label="Cap (USD)">
                <Input mono value={cap} onChange={setCap} />
              </Field>
              <Field label="Window">
                <Select options={["daily", "weekly", "monthly", "total"]} value={win} onChange={(v) => setWin(v as BudgetWindow)} />
              </Field>
              <div style={{ gridColumn: "1 / -1" }}>
                <Field hint={block ? "Requests over the cap are refused with 402 budget_exceeded." : "Over-cap requests continue; the key is flagged on the dashboard."}>
                  <Switch checked={block} onChange={setBlock} label="Block requests over cap" />
                </Field>
              </div>
            </div>
          ) : (
            <p style={{ margin: 0, color: "var(--fg-3)", font: "var(--w-reg) var(--fs-12)/1.5 var(--font-sans)" }}>
              Uncapped keys still meter spend — you can add a cap later without reissuing.
            </p>
          )}
        </div>
      )}
    </Dialog>
  );
}

export interface KeysScreenProps {
  data: LoomSnapshot;
  role: "operator" | "tenant";
  tenant: string;
}

export function KeysScreen({ data, role, tenant }: KeysScreenProps) {
  const client = useLoom();
  const formatMoney = Fmt.money;
  const [query, setQuery] = useState("");
  const [creating, setCreating] = useState(false);
  const [revoking, setRevoking] = useState<VirtualKey | null>(null);
  const [keys, setKeys] = useState<VirtualKey[]>(data.keys);
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
    const updated = await client.revokeKey(revoking.id);
    setKeys(keys.map((k) => (k.id === updated.id ? updated : k)));
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
