// ProviderDetail — one provider's credential, base-URL override, connectivity
// probe and the per-tenant credential overrides that bill to their own account.
import { useState } from "react";
import {
  Card, Field, Input, SecretInput, Button, Badge, DataTable, Dialog, StatusDot, type Column,
} from "../components/index.ts";
import { useLoom } from "../api/context.tsx";
import type { LoomSnapshot, Provider, CredOverride, ConnectivityResult } from "../api/types.ts";

/** Props for {@link ProviderDetail}. */
export interface ProviderDetailProps {
  readonly data: LoomSnapshot;
  /** The provider being inspected. */
  readonly p: Provider;
}

/** Renders one provider's credential settings, connectivity check and overrides. */
export function ProviderDetail({ data, p }: ProviderDetailProps) {
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
