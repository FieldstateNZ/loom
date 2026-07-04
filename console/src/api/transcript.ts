// The transcript block system — the discriminated union the conversation
// renderer walks. Blocks are DTOs (all `readonly`); the HTTP client assembles
// them from provider JSON using local mutable drafts, then hands back readonly
// values (see api/transcript-map.ts).

/** Assistant/user prose. */
export interface TextBlock {
  readonly type: "text";
  readonly text: string;
}

/** Extended-reasoning content, rendered collapsed. */
export interface ThinkingBlock {
  readonly type: "thinking";
  readonly duration?: string;
  readonly text: string;
}

/** A cache read/write marker, surfaced from the turn's usage counts. */
export interface CacheBlock {
  readonly type: "cache";
  readonly kind: "read" | "write";
  readonly tokens: number;
}

/** A tool call plus its correlated result (merged into one block). */
export interface ToolUseBlock {
  readonly type: "tool_use";
  readonly name: string;
  /** Where the tool ran (`"server"`, an MCP server name, etc.). */
  readonly via?: string;
  readonly input?: unknown;
  readonly result?: unknown;
  readonly isError?: boolean;
}

/** One web-search hit. `cited` marks results the model actually referenced. */
export interface WebSearchResult {
  readonly title: string;
  readonly url: string;
  readonly snippet?: string;
  readonly cited?: boolean;
}

/** A server-side web search: the query and its (result) hits. */
export interface WebSearchBlock {
  readonly type: "web_search";
  readonly query: string;
  readonly results: readonly WebSearchResult[];
}

/** A server-side code execution: source plus stdout/stderr/exit code. */
export interface CodeExecBlock {
  readonly type: "code_exec";
  readonly lang?: string;
  readonly code?: string;
  readonly stdout?: string;
  readonly stderr?: string;
  readonly exitCode?: number;
}

/**
 * Forward-compatible fallback: any block type the console does not model is
 * preserved verbatim and rendered as raw JSON, so a new provider block never
 * breaks the transcript.
 */
export interface UnknownBlock {
  readonly type: string;
  readonly blockType?: string;
  readonly data?: unknown;
  readonly [key: string]: unknown;
}

/** Every content block a turn can contain. */
export type TranscriptBlock =
  | TextBlock
  | ThinkingBlock
  | CacheBlock
  | ToolUseBlock
  | WebSearchBlock
  | CodeExecBlock
  | UnknownBlock;

/** Per-turn token/cost/latency usage. */
export interface TurnUsage {
  readonly cost?: number;
  readonly inTok?: number;
  readonly outTok?: number;
  readonly cacheRead?: number;
  readonly cacheWrite?: number;
  readonly ms?: number;
}

/** One turn (message) in a transcript. */
export interface TranscriptTurn {
  readonly role: "user" | "assistant" | "system";
  readonly time?: string;
  readonly model?: string;
  readonly usage?: TurnUsage;
  readonly blocks: readonly TranscriptBlock[];
}

/** A full turn-by-turn conversation transcript with rolled-up totals. */
export interface Transcript {
  readonly id: string;
  readonly key: string;
  readonly model: string;
  readonly totals: {
    readonly cost: number;
    readonly inTok: number;
    readonly outTok: number;
    readonly cacheRead: number;
    readonly cacheWrite: number;
  };
  readonly turns: readonly TranscriptTurn[];
}
