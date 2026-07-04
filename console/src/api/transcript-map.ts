// Maps loom-core `Message` / `ContentPart` JSON into the console's transcript
// block model. Tool calls and their results arrive in separate messages, so a
// `pending` map correlates them by id and merges the result back into the call.
// Blocks are public `readonly` DTOs; this module assembles them via local
// mutable drafts (`Writable`) and hands back the readonly values.
import { num, asRecord, str } from "./json.ts";
import { parseWebSearchResults, applyCodeExecResult } from "./transcript-results.ts";
import type { Writable } from "./writable.ts";
import type {
  TranscriptTurn,
  TranscriptBlock,
  ToolUseBlock,
  WebSearchBlock,
  CodeExecBlock,
  TurnUsage,
} from "./transcript.ts";

/** A block awaiting its correlated `*_result` part. */
export type PendingBlock =
  | Writable<ToolUseBlock>
  | Writable<WebSearchBlock>
  | Writable<CodeExecBlock>;

const WEB_SEARCH_NAMES = /web_search/i;
const CODE_EXEC_NAMES = /code_(execution|interpreter)|bash_code/i;

/** Maps one gateway `Message` into a console transcript turn. */
export function mapMessage(
  msg: Record<string, unknown>,
  convModel: string,
  pending: Map<string, PendingBlock>,
): TranscriptTurn {
  const rawRole = str(msg.role);
  const role: TranscriptTurn["role"] =
    rawRole === "user" ? "user" : rawRole === "assistant" ? "assistant" : "system";

  const usage = mapUsage(asRecord(msg.usage));
  const blocks: TranscriptBlock[] = [];

  // Surface the turn's cache read/write as markers, from real usage counts.
  if (usage?.cacheWrite) blocks.push({ type: "cache", kind: "write", tokens: usage.cacheWrite });
  if (usage?.cacheRead) blocks.push({ type: "cache", kind: "read", tokens: usage.cacheRead });

  const parts = Array.isArray(msg.content) ? (msg.content as unknown[]) : [];
  for (const raw of parts) {
    const part = asRecord(raw);
    if (part) mapPart(part, blocks, pending);
  }

  return {
    role,
    blocks,
    ...(role === "assistant" && convModel ? { model: convModel } : {}),
    ...(usage ? { usage } : {}),
  };
}

/** Extracts per-turn token usage, or `undefined` when the message reports none. */
function mapUsage(u: Record<string, unknown> | null): TurnUsage | undefined {
  if (!u) return undefined;
  const usage: Writable<TurnUsage> = {};
  if (u.input_tokens != null) usage.inTok = num(u.input_tokens);
  if (u.output_tokens != null) usage.outTok = num(u.output_tokens);
  if (u.cache_read_tokens != null) usage.cacheRead = num(u.cache_read_tokens);
  if (u.cache_write_tokens != null) usage.cacheWrite = num(u.cache_write_tokens);
  return Object.keys(usage).length > 0 ? usage : undefined;
}

/** Maps one ContentPart into 0..1 transcript blocks (mutating `blocks`). */
function mapPart(
  part: Record<string, unknown>,
  blocks: TranscriptBlock[],
  pending: Map<string, PendingBlock>,
): void {
  const type = str(part.type) ?? "unknown";
  switch (type) {
    case "text":
      return void blocks.push({ type: "text", text: str(part.text) ?? "" });
    case "thinking":
      return void blocks.push({ type: "thinking", text: str(part.thinking) ?? "" });
    case "redacted_thinking":
      return void blocks.push({ type: "thinking", text: "[redacted reasoning]" });
    case "tool_use": {
      const block: Writable<ToolUseBlock> = { type: "tool_use", name: str(part.name) ?? "tool", input: part.input };
      const id = str(part.id);
      if (id) pending.set(id, block);
      blocks.push(block);
      return;
    }
    case "tool_result":
      return mapToolResult(part, blocks, pending);
    case "server_tool_use":
      return mapServerToolUse(part, blocks, pending);
    case "server_tool_result":
      return mapServerToolResult(part, blocks, pending);
    case "provider_extension":
      return void blocks.push({ type: "unknown", blockType: str(part.kind) ?? "provider_extension", data: part.payload ?? part });
    default:
      // image, document, and any future/unmodelled part — preserved verbatim.
      return void blocks.push({ type: "unknown", blockType: type, data: part });
  }
}

/** Merges a `tool_result` back into its pending call, or renders it standalone. */
function mapToolResult(
  part: Record<string, unknown>,
  blocks: TranscriptBlock[],
  pending: Map<string, PendingBlock>,
): void {
  const id = str(part.tool_use_id);
  const target = id ? pending.get(id) : undefined;
  if (target && target.type === "tool_use") {
    target.result = part.content;
    if (part.is_error != null) target.isError = Boolean(part.is_error);
    return;
  }
  blocks.push({
    type: "tool_use",
    name: "tool_result",
    result: part.content,
    ...(part.is_error != null ? { isError: Boolean(part.is_error) } : {}),
  });
}

/** Classifies a `server_tool_use` into a web-search, code-exec, or plain tool block. */
function mapServerToolUse(
  part: Record<string, unknown>,
  blocks: TranscriptBlock[],
  pending: Map<string, PendingBlock>,
): void {
  const name = str(part.name) ?? "server_tool";
  const id = str(part.id);
  const input = asRecord(part.input);
  let block: PendingBlock;
  if (WEB_SEARCH_NAMES.test(name)) {
    block = { type: "web_search", query: (input && str(input.query)) ?? "", results: [] };
  } else if (CODE_EXEC_NAMES.test(name)) {
    const code = input ? str(input.code) : undefined;
    const lang = input ? str(input.language) : undefined;
    block = { type: "code_exec", ...(code ? { code } : {}), ...(lang ? { lang } : {}) };
  } else {
    block = { type: "tool_use", name, via: "server", input: part.input };
  }
  if (id) pending.set(id, block);
  blocks.push(block);
}

/** Merges a `server_tool_result` into its pending web-search/code-exec/tool block. */
function mapServerToolResult(
  part: Record<string, unknown>,
  blocks: TranscriptBlock[],
  pending: Map<string, PendingBlock>,
): void {
  const id = str(part.tool_use_id);
  const target = id ? pending.get(id) : undefined;
  if (target?.type === "web_search") target.results = parseWebSearchResults(part.content);
  else if (target?.type === "code_exec") applyCodeExecResult(target, part.content);
  else if (target?.type === "tool_use") target.result = part.content;
  else blocks.push({ type: "unknown", blockType: "server_tool_result", data: part });
}
