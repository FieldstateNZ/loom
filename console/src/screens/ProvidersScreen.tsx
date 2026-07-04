// ProvidersScreen — providers as a collection (Anthropic is the first, not the only
// shape): list → provider detail (credential, base URL, connectivity, overrides).
import { useState } from "react";
import {
  Card, Field, Input, SecretInput, Button, Badge, DataTable, Dialog, StatusDot, Select,
  type Column,
} from "../components/index.ts";
import { useLoom } from "../api/context.tsx";
import type { LoomSnapshot, Provider, CredOverride, ConnectivityResult } from "../api/types.ts";

function ProviderDetail({ data, p }: { data: LoomSnapshot; p: Provider }) {
  const client = useLoom();
  const [checking, setChecking] = useState(false);
  const [checkResult, setCheckResult] = useState<ConnectivityResult | null>(null);
  const [override, setOverride] = useState<CredOverride | null>(null);
  const overrides = data.credOverrides.filter((o) => o.provider === p.id);
  const runCheck = async () => {
    setChecking(true); setCheckResult(null);
    const res = await client.checkProviderConnectivity(p.id);
    setChecking(false); setCheckResult(res);
  };

  const columns: Column<CredOverride>[] = [
    { key: "tenant", label: "Tenant", mono: true },
    { key: "state", label: "Credential", render: (r) => r.set ? <Badge tone="accent" caps>Override</Badge> : <Badge caps>Inherits</Badge> },
    { key: "meta", label: "Last rotated", muted: true, render: (r) => r.meta || "—" },
    { key: "action", label: "", align: "right", width: "110px", render: (r) => <Button size="sm" variant="ghost" icon={r.set ? "rotate-cw" : "plus"} onClick={() => setOverride(r)}>{r.set ? "Rotate" : "Set"}</Button> },
  ];

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "12px", maxWidth: "760px" }}>
      <Card eyebrow={"Credential — " + p.id}
        actions={<StatusDot tone={p.status === "connected" ? "ok" : "danger"} label={p.status} />}
        footer={<span>Used by every tenant without an override · last check {p.lastCheck}</span>}>
        <div style={{ display: "flex", flexDirection: "column", gap: "14px" }}>
          <Field label="API key">
            <SecretInput isSet meta={p.keyMeta} />
          </Field>
          <Field label="Base URL override" hint={"Leave empty for " + p.defaultBaseUrl.replace("https://", "") + "."}>
            <Input mono placeholder={p.defaultBaseUrl} value={p.baseUrl || ""} onChange={() => {}} />
          </Field>
          <div style={{ display: "flex", gap: "8px", alignItems: "center" }}>
            <Button icon="plug" loading={checking} onClick={() => void runCheck()}>Check connectivity</Button>
            {checkResult ? (
              <span style={{ font: "var(--w-reg) var(--fs-12)/1 var(--font-mono)", color: checkResult.ok ? "var(--ok)" : "var(--danger)" }}>{checkResult.detail}</span>
            ) : null}
          </div>
        </div>
      </Card>
      <Card eyebrow="Per-tenant overrides" flush footer={<span>Overrides bill a tenant to its own {p.name} account; everything else inherits this credential.</span>}>
        <DataTable rowKey="tenant" columns={columns} rows={overrides} />
      </Card>
      <Dialog open={!!override} onClose={() => setOverride(null)} title={override ? (override.set ? "Rotate override — " : "Set override — ") + override.tenant : ""} icon="shield" width={480}
        footer={<Button variant="ghost" onClick={() => setOverride(null)}>Close</Button>}>
        <Field label={p.name + " API key"} hint="Write-only. Requests from this tenant switch to the new key within 60 seconds.">
          <SecretInput placeholder="sk-ant-api03-…" saveLabel="Save & check" onSave={() => setOverride(null)} />
        </Field>
      </Dialog>
    </div>
  );
}

export interface ProvidersScreenProps {
  data: LoomSnapshot;
  detailId: string | null;
  onOpenProvider: (p: Provider) => void;
}

export function ProvidersScreen({ data, detailId, onOpenProvider }: ProvidersScreenProps) {
  const [adding, setAdding] = useState(false);
  const detail = detailId ? data.providers.find((p) => p.id === detailId) : undefined;
  if (detail) return <ProviderDetail data={data} p={detail} />;

  const columns: Column<Provider>[] = [
    { key: "name", label: "Provider" },
    { key: "api", label: "API", render: (r) => <Badge tone={r.api === "native" ? "accent" : "neutral"} caps>{r.api}</Badge> },
    { key: "status", label: "Status", render: (r) => <StatusDot tone={r.status === "connected" ? "ok" : "danger"} label={r.status} /> },
    { key: "cred", label: "Credential", muted: true, render: (r) => r.keyMeta },
    { key: "base", label: "Base URL", mono: true, muted: true, render: (r) => r.baseUrl || "default" },
    { key: "overrides", label: "Tenant overrides", align: "right", mono: true, render: (r) => String(data.credOverrides.filter((o) => o.provider === r.id && o.set).length) },
    { key: "models", label: "Models", align: "right", mono: true },
  ];

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "12px" }}>
      <div style={{ display: "flex", justifyContent: "flex-end" }}>
        <Button variant="primary" icon="plus" size="sm" onClick={() => setAdding(true)}>Add provider</Button>
      </div>
      <Card flush footer={<span>Loom speaks Anthropic natively today; translated providers register here as translation layers land.</span>}>
        <DataTable rowKey="id" onRowClick={(r) => onOpenProvider(r)} columns={columns} rows={data.providers} />
      </Card>
      <Dialog open={adding} onClose={() => setAdding(false)} title="Add provider" width={480}
        footer={<>
          <Button variant="ghost" onClick={() => setAdding(false)}>Cancel</Button>
          <Button variant="primary" onClick={() => setAdding(false)}>Add & check</Button>
        </>}>
        <div style={{ display: "flex", flexDirection: "column", gap: "14px" }}>
          <Field label="Provider" hint="v1 ships the native Anthropic layer; more appear here as translation layers land.">
            <Select options={[{ value: "anthropic", label: "Anthropic — native" }]} value="anthropic" onChange={() => {}} />
          </Field>
          <Field label="API key">
            <SecretInput placeholder="sk-ant-api03-…" saveLabel="Attach" />
          </Field>
          <Field label="Base URL override" hint="Optional — for proxies or regional endpoints.">
            <Input mono placeholder="https://api.anthropic.com" value="" onChange={() => {}} />
          </Field>
        </div>
      </Dialog>
    </div>
  );
}
