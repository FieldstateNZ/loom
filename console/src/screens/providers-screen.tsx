// ProvidersScreen — providers as a collection (Anthropic is the first, not the only
// shape): list → provider detail (credential, base URL, connectivity, overrides).
import { useState } from "react";
import {
  Card, Field, Input, SecretInput, Button, Badge, DataTable, Dialog, StatusDot, Select,
  type Column,
} from "../components/index.ts";
import { ProviderDetail } from "./provider-detail.tsx";
import type { LoomSnapshot, Provider } from "../api/types.ts";

/** Props for {@link ProvidersScreen}. */
export interface ProvidersScreenProps {
  readonly data: LoomSnapshot;
  /** The provider id being drilled into, or `null` for the list view. */
  readonly detailId: string | null;
  readonly onOpenProvider: (p: Provider) => void;
}

/** Provider list; renders {@link ProviderDetail} when one is selected. */
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
