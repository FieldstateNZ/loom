// Best-effort extraction of server-tool results (web search, code execution)
// from provider-native payloads whose exact shape varies. Both functions favour
// resilience over strictness: anything unrecognised is simply skipped.
import { num, asRecord, str } from "./json.ts";
import type { CodeExecBlock, WebSearchResult } from "./transcript.ts";
import type { Writable } from "./writable.ts";

/** Extracts web-search hits, surfacing only genuine snippets (never blobs). */
export function parseWebSearchResults(content: unknown): WebSearchResult[] {
  const nested = asRecord(content)?.content;
  const items = Array.isArray(content) ? content : Array.isArray(nested) ? nested : [];
  const out: WebSearchResult[] = [];
  for (const raw of items) {
    const r = asRecord(raw);
    if (!r) continue;
    const url = str(r.url);
    const title = str(r.title);
    if (!url && !title) continue;
    const snippet = str(r.snippet) ?? str(r.description);
    out.push({ title: title ?? url ?? "", url: url ?? "", ...(snippet ? { snippet } : {}) });
  }
  return out;
}

/** Fills stdout/stderr/exit code onto a code-exec draft from its result payload. */
export function applyCodeExecResult(block: Writable<CodeExecBlock>, content: unknown): void {
  const r = asRecord(content) ?? asRecord(asRecord(content)?.content);
  if (!r) return;
  const stdout = str(r.stdout);
  const stderr = str(r.stderr);
  if (stdout != null) block.stdout = stdout;
  if (stderr != null) block.stderr = stderr;
  if (r.return_code != null) block.exitCode = num(r.return_code);
  else if (r.exit_code != null) block.exitCode = num(r.exit_code);
}
