// McpScreen — register/edit MCP servers with write-only tokens + connectivity check.
import { useEffect, useState } from "react";
import {
  Card, DataTable, StatusDot, Button, Input, Dialog, Field, SecretInput, EmptyState,
  type Column,
} from "../components/index.ts";
import { useLoom } from "../api/context.tsx";
import type { LoomSnapshot, McpServer, ConnectivityResult } from "../api/types.ts";

function McpEditDialog({ server, onClose }: { server: McpServer | null; onClose: () => void }) {
  const client = useLoom();
  const [url, setUrl] = useState(server ? server.url : "");
  const [checking, setChecking] = useState(false);
  const [checkResult, setCheckResult] = useState<ConnectivityResult | null>(null);
  useEffect(() => { if (server) { setUrl(server.url); setCheckResult(null); setChecking(false); } }, [server]);
  if (!server) return null;
  const runCheck = async () => {
    setChecking(true); setCheckResult(null);
    const res = await client.checkMcpConnectivity(server.id);
    setChecking(false); setCheckResult(res);
  };
  return (
    <Dialog open onClose={onClose} title={"Edit " + server.name} icon="server" width={480}
      footer={<>
        <Button variant="danger-secondary" style={{ marginRight: "auto" }}>Remove server</Button>
        <Button variant="ghost" onClick={onClose}>Cancel</Button>
        <Button variant="primary" onClick={onClose}>Save</Button>
      </>}>
      <div style={{ display: "flex", flexDirection: "column", gap: "14px" }}>
        <Field label="URL">
          <Input mono value={url} onChange={setUrl} />
        </Field>
        <Field label="Bearer token">
          <SecretInput isSet meta={server.tokenMeta} />
        </Field>
        <div style={{ display: "flex", gap: "10px", alignItems: "center" }}>
          <Button icon="plug" loading={checking} onClick={() => void runCheck()}>Check connectivity</Button>
          {checkResult ? (
            <span style={{ font: "var(--w-reg) var(--fs-12)/1 var(--font-mono)", color: checkResult.ok ? "var(--ok)" : "var(--danger)" }}>{checkResult.detail}</span>
          ) : null}
        </div>
      </div>
    </Dialog>
  );
}

export interface McpScreenProps {
  data: LoomSnapshot;
  role: "operator" | "tenant";
  tenant: string;
}

export function McpScreen({ data, role, tenant }: McpScreenProps) {
  const [adding, setAdding] = useState(false);
  const [editing, setEditing] = useState<McpServer | null>(null);
  const [name, setName] = useState("");
  const [url, setUrl] = useState("");
  const rows = data.mcpServers.filter((m) => role !== "tenant" || m.tenant === tenant);

  const columns: Column<McpServer>[] = [
    { key: "name", label: "Server", mono: true },
    ...(role === "operator" ? [{ key: "tenant", label: "Tenant", muted: true } as Column<McpServer>] : []),
    { key: "url", label: "URL", mono: true, muted: true },
    { key: "status", label: "Auth", render: (r) => <StatusDot tone={r.status === "connected" ? "ok" : "danger"} label={r.status} /> },
    { key: "last", label: "Last used", mono: true, muted: true },
    { key: "token", label: "Token", muted: true, render: () => <span style={{ font: "var(--w-reg) var(--fs-11)/1 var(--font-mono)", color: "var(--fg-4)" }}>•••• write-only</span> },
    { key: "edit", label: "", align: "right", width: "80px", render: (r) => <Button size="sm" variant="ghost" icon="pencil" onClick={() => setEditing(r)}>Edit</Button> },
  ];

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "12px" }}>
      <div style={{ display: "flex", justifyContent: "flex-end" }}>
        <Button variant="primary" icon="plus" size="sm" onClick={() => setAdding(true)}>Add server</Button>
      </div>
      <Card flush footer={<span>Tokens are write-only — Loom stores them encrypted and never displays them back.</span>}>
        <DataTable
          rowKey="id"
          onRowClick={(r) => setEditing(r)}
          columns={columns}
          rows={rows}
          empty={<EmptyState icon="server" title="No MCP servers registered"
            hint="Register a server so conversations can reference its tools by name."
            action={<Button variant="primary" icon="plus" onClick={() => setAdding(true)}>Add server</Button>} />}
        />
      </Card>
      <Dialog open={adding} onClose={() => setAdding(false)} title="Register MCP server" width={480}
        footer={<>
          <Button variant="ghost" onClick={() => setAdding(false)}>Cancel</Button>
          <Button variant="primary" disabled={!name || !url} onClick={() => setAdding(false)}>Register</Button>
        </>}>
        <div style={{ display: "flex", flexDirection: "column", gap: "14px" }}>
          <Field label="Name" hint="Conversations reference the server by this name." required>
            <Input mono value={name} onChange={setName} placeholder="github-mcp" />
          </Field>
          <Field label="URL" required>
            <Input mono value={url} onChange={setUrl} placeholder="https://mcp.internal.fieldstate.nz/github" />
          </Field>
          <Field label="Bearer token">
            <SecretInput placeholder="Paste token…" saveLabel="Attach" />
          </Field>
        </div>
      </Dialog>
      <McpEditDialog server={editing} onClose={() => setEditing(null)} />
    </div>
  );
}
