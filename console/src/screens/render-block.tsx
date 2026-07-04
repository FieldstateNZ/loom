// Maps a transcript block DTO to the component that renders it. Kept apart from
// the screen so the block dispatch is one focused, testable function.
import type { ReactNode } from "react";
import {
  BlockText, BlockThinking, BlockToolUse, BlockWebSearch, BlockCodeExec, BlockUnknown, CacheMarker,
} from "../components/index.ts";
import type {
  TranscriptBlock, TextBlock, ThinkingBlock, ToolUseBlock, WebSearchBlock, CodeExecBlock, CacheBlock, UnknownBlock,
} from "../api/types.ts";

/**
 * Renders one transcript block to its matching component (`i` is the React list
 * key). The `default` case is the forward-compatible raw-JSON fallback, so an
 * unrecognised block type renders safely instead of breaking the transcript.
 */
export function renderBlock(block: TranscriptBlock, i: number): ReactNode {
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
