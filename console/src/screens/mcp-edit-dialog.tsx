// McpEditDialog — edit a registered MCP server's URL + write-only bearer token,
// with a live connectivity check.
import { useEffect, useState } from "react";
import { Button, Input, Dialog, Field, SecretInput } from "../components/index.ts";
import { useLoom } from "../api/context.tsx";
import type { McpServer, ConnectivityResult } from "../api/types.ts";

/** Props for {@link McpEditDialog}. */
export interface McpEditDialogProps {
  /** The server to edit; `null` closes the dialog. */
  readonly server: McpServer | null;
  readonly onClose: () => void;
}

/** Edit dialog for one MCP server (URL, token, connectivity probe). */
export function McpEditDialog({ server, onClose }: McpEditDialogProps) {
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
