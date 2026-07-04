// ConversationsScreen — searchable list + turn-by-turn transcript renderer.
import { useEffect, useState } from "react";
import {
  Card, Input, Badge, Transcript, Turn, IconButton, EmptyState,
  BlockText, BlockThinking, BlockToolUse, BlockWebSearch, BlockCodeExec, BlockUnknown, CacheMarker,
} from "../components/index.ts";
import { Fmt } from "../lib/format.ts";
import { useLoom } from "../api/context.tsx";
import type {
  LoomSnapshot, Transcript as TranscriptData, TranscriptBlock,
  TextBlock, ThinkingBlock, ToolUseBlock, WebSearchBlock, CodeExecBlock, CacheBlock, UnknownBlock,
} from "../api/types.ts";

function renderBlock(block: TranscriptBlock, i: number) {
  switch (block.type) {
    case "text": return <BlockText key={i}>{(block as TextBlock).text}</BlockText>;
    case "thinking": { const b = block as ThinkingBlock; return <BlockThinking key={i} duration={b.duration}>{b.text}</BlockThinking>; }
    case "tool_use": { const b = block as ToolUseBlock; return <BlockToolUse key={i} name={b.name} via={b.via} input={b.input} result={b.result} isError={b.isError} />; }
    case "web_search": { const b = block as WebSearchBlock; return <BlockWebSearch key={i} query={b.query} results={b.results} />; }
    case "code_exec": { const b = block as CodeExecBlock; return <BlockCodeExec key={i} lang={b.lang} code={b.code} stdout={b.stdout} stderr={b.stderr} exitCode={b.exitCode} />; }
    case "cache": { const b = block as CacheBlock; return <CacheMarker key={i} kind={b.kind} tokens={b.tokens} />; }
    default: { const b = block as UnknownBlock; return <BlockUnknown key={i} type={b.blockType || b.type} data={b.data ?? b} />; }
  }
}

export interface ConversationsScreenProps {
  data: LoomSnapshot;
  role: "operator" | "tenant";
  tenant: string;
}

export function ConversationsScreen({ data, role, tenant }: ConversationsScreenProps) {
  const client = useLoom();
  const formatMoney = Fmt.money, formatTokens = Fmt.tokens;
  const [query, setQuery] = useState("");
  const [selectedId, setSelectedId] = useState("conv_9f2c4e8a");
  const [detail, setDetail] = useState<TranscriptData | null>(null);

  useEffect(() => {
    let live = true;
    client.getTranscript(selectedId).then((t) => { if (live) setDetail(t); });
    return () => { live = false; };
  }, [client, selectedId]);

  const keysByTenant = new Set(data.keys.filter((k) => role !== "tenant" || k.tenant === tenant).map((k) => k.name));
  const convs = data.conversations
    .filter((c) => keysByTenant.has(c.key))
    .filter((c) => !query || c.preview.toLowerCase().includes(query.toLowerCase()) || c.id.includes(query.toLowerCase()));

  return (
    <div style={{ display: "grid", gridTemplateColumns: "300px minmax(0, 1fr)", gap: "12px", alignItems: "start" }}>
      <div style={{ display: "flex", flexDirection: "column", gap: "8px" }}>
        <Input icon="search" size="sm" placeholder="Search conversations…" value={query} onChange={setQuery} />
        <Card flush>
          <div>
            {convs.map((c) => (
              <button key={c.id} type="button" onClick={() => setSelectedId(c.id)}
                style={{
                  display: "block", width: "100%", textAlign: "left", cursor: "pointer",
                  background: c.id === selectedId ? "var(--bg-2)" : "transparent",
                  border: 0, borderBottom: "1px solid var(--border-1)", padding: "10px 12px",
                  transition: "background var(--dur-1) var(--ease-out)",
                }}>
                <span style={{ display: "flex", gap: "8px", alignItems: "baseline", marginBottom: "4px" }}>
                  <span style={{ font: "var(--w-med) var(--fs-11)/1 var(--font-mono)", color: "var(--fg-1)" }}>{c.id}</span>
                  <span style={{ font: "var(--w-reg) 10px/1 var(--font-mono)", color: "var(--fg-4)", marginLeft: "auto" }}>{c.last}</span>
                </span>
                <span style={{
                  display: "-webkit-box", WebkitLineClamp: 2, WebkitBoxOrient: "vertical", overflow: "hidden",
                  font: "var(--w-reg) var(--fs-12)/1.4 var(--font-sans)", color: "var(--fg-3)", marginBottom: "6px",
                }}>{c.preview}</span>
                <span style={{ display: "flex", gap: "6px", alignItems: "center", font: "var(--w-reg) 10px/1 var(--font-mono)", color: "var(--fg-4)" }}>
                  <span>{c.key}</span>
                  <span>·</span>
                  <span>{c.turns} turns</span>
                  <span style={{ marginLeft: "auto", color: "var(--fg-2)" }}>{formatMoney(c.cost)}</span>
                </span>
              </button>
            ))}
          </div>
        </Card>
      </div>
      {detail ? (
        <Card flush>
          <div style={{ display: "flex", alignItems: "center", gap: "10px", padding: "12px 16px", borderBottom: "1px solid var(--border-1)" }}>
            <span style={{ font: "var(--w-med) var(--fs-13)/1 var(--font-mono)", color: "var(--fg-1)" }}>{detail.id}</span>
            <Badge>{detail.key}</Badge>
            <Badge>{detail.model}</Badge>
            <span style={{ marginLeft: "auto", font: "var(--w-reg) var(--fs-11)/1 var(--font-mono)", color: "var(--fg-3)" }}>
              {formatMoney(detail.totals.cost)} · {formatTokens(detail.totals.inTok)} in · {formatTokens(detail.totals.outTok)} out · cache r {formatTokens(detail.totals.cacheRead)}
            </span>
            <IconButton icon="download" label="Export JSON" size="sm" />
          </div>
          <div style={{ padding: "18px 20px 22px" }}>
            <Transcript>
              {detail.turns.map((turn, i) => (
                <Turn key={i} role={turn.role} time={turn.time} model={turn.model} usage={turn.usage}>
                  {turn.blocks.map(renderBlock)}
                </Turn>
              ))}
            </Transcript>
          </div>
        </Card>
      ) : (
        <EmptyState icon="message-square" title="Transcript not loaded in this mock"
          hint="Select conv_9f2c4e8a to see the full turn-by-turn renderer — every block type, including the unknown-block fallback."
          style={{ border: "1px solid var(--border-1)", minHeight: "300px" }} />
      )}
    </div>
  );
}
