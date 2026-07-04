// The create-key flow: name → scopes → budget → the shown-once secret moment.
import { useState } from "react";
import { Badge, Button, Dialog, Field, Input, Select, Switch, RevealOnce } from "../components/index.ts";
import { useLoom } from "../api/context.tsx";
import type { VirtualKey, Tenant, CreateKeyInput, BudgetWindow } from "../api/types.ts";

/** Props for {@link CreateKeyDialog}. */
export interface CreateKeyDialogProps {
  readonly open: boolean;
  readonly onClose: () => void;
  /** Called with the new key once the operator confirms they stored the secret. */
  readonly onCreated: (key: VirtualKey) => void;
  readonly tenants: readonly Tenant[];
}

/** Multi-step dialog that issues a virtual key and reveals its secret exactly once. */
export function CreateKeyDialog({ open, onClose, onCreated, tenants }: CreateKeyDialogProps) {
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
  const [error, setError] = useState<string | null>(null);
  const [secret, setSecret] = useState<string | null>(null);
  const [created, setCreated] = useState<VirtualKey | null>(null);

  const reset = () => {
    setStep(0); setName(""); setScopes({ messages: true, streaming: true, mcp: false });
    setCapOn(true); setCap("50"); setWin("daily"); setBlock(true);
    setBusy(false); setError(null); setSecret(null); setCreated(null);
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
    setError(null);
    const res = await client.createKey(input);
    setBusy(false);
    if (!res.ok) { setError(res.error.message); return; }
    setSecret(res.value.secret);
    setCreated(res.value.key);
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
          {error ? (
            <p style={{ margin: 0, color: "var(--danger)", font: "var(--w-reg) var(--fs-12)/1.5 var(--font-sans)" }}>{error}</p>
          ) : null}
        </div>
      )}
    </Dialog>
  );
}
