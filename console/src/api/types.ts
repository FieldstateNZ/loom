// Loom admin/usage API — domain types.
//
// These model the JSON the gateway's REST API returns. They are the contract
// the console codes against; the mock client (mock.ts) and any future HTTP
// client both satisfy the LoomClient interface in client.ts against these
// shapes, so swapping the implementation is a one-file change.

export type KeyStatus = "active" | "blocked" | "revoked";
export type BudgetWindow = "daily" | "weekly" | "monthly" | "total";
export type BudgetMode = "block" | "warn";
export type Scope = "messages" | "streaming" | "mcp";

export interface VirtualKey {
  id: string;
  name: string;
  tenant: string;
  status: KeyStatus;
  scopes: string[];
  budgetSpent: number;
  cap: number | null;
  window: BudgetWindow | null;
  mode: BudgetMode;
  last: string;
  spend7: number;
  rateRpm?: number;
}

export interface Tenant {
  id: string;
  name: string;
  status: "active" | "suspended";
  keys: number;
  mcp: number;
  spend30: number;
  cap: number | null;
  window: BudgetWindow;
  share: number;
  requests30: number;
  blocks30: number;
}

export interface CredOverride {
  tenant: string;
  provider: string;
  set: boolean;
  meta: string | null;
  baseUrl: string | null;
}

export interface Provider {
  id: string;
  name: string;
  api: "native" | "translated";
  status: "connected" | "error";
  keyMeta: string;
  baseUrl: string | null;
  defaultBaseUrl: string;
  models: number;
  lastCheck: string;
}

export interface GatewayStats {
  spendToday: number;
  spendPrior: number;
  spendDelta: number;
  tokensIn: number;
  tokensInDelta: number;
  tokensOut: number;
  tokensOutDelta: number;
  requests: number;
  requestsDelta: number;
  streams: number;
  cacheReadToday: number;
  cacheWriteToday: number;
  cacheSavedToday: number;
  cacheHitRate: number;
}

export interface UsageDaily {
  labels: string[];
  cost: number[];
  input: number[];
  output: number[];
  cacheRead: number[];
  cacheWrite: number[];
}

export interface BarItem {
  label: string;
  value: number;
  display?: string;
  color?: string;
  key?: string;
}

export type EventTone = "danger" | "warn";
export interface GatewayEvent {
  time: string;
  kind: "block" | "error" | "warn";
  tone: EventTone;
  key: string;
  detail: string;
}

export interface UsageByKey {
  key: string;
  requests: number;
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  cost: number;
}

export interface McpServer {
  id: string;
  tenant: string;
  name: string;
  url: string;
  status: "connected" | "error";
  last: string;
  tokenMeta: string;
}

export interface Conversation {
  id: string;
  key: string;
  model: string;
  turns: number;
  last: string;
  cost: number;
  tokens: number;
  preview: string;
  blocked?: boolean;
}

// ── Transcript block system ────────────────────────────────────────────────

export interface TextBlock {
  type: "text";
  text: string;
}
export interface ThinkingBlock {
  type: "thinking";
  duration?: string;
  text: string;
}
export interface CacheBlock {
  type: "cache";
  kind: "read" | "write";
  tokens: number;
}
export interface ToolUseBlock {
  type: "tool_use";
  name: string;
  via?: string;
  input?: unknown;
  result?: unknown;
  isError?: boolean;
}
export interface WebSearchResult {
  title: string;
  url: string;
  snippet?: string;
  cited?: boolean;
}
export interface WebSearchBlock {
  type: "web_search";
  query: string;
  results: WebSearchResult[];
}
export interface CodeExecBlock {
  type: "code_exec";
  lang?: string;
  code?: string;
  stdout?: string;
  stderr?: string;
  exitCode?: number;
}
export interface UnknownBlock {
  type: string;
  blockType?: string;
  data?: unknown;
  [key: string]: unknown;
}

export type TranscriptBlock =
  | TextBlock
  | ThinkingBlock
  | CacheBlock
  | ToolUseBlock
  | WebSearchBlock
  | CodeExecBlock
  | UnknownBlock;

export interface TurnUsage {
  cost?: number;
  inTok?: number;
  outTok?: number;
  cacheRead?: number;
  cacheWrite?: number;
  ms?: number;
}

export interface TranscriptTurn {
  role: "user" | "assistant" | "system";
  time?: string;
  model?: string;
  usage?: TurnUsage;
  blocks: TranscriptBlock[];
}

export interface Transcript {
  id: string;
  key: string;
  model: string;
  totals: { cost: number; inTok: number; outTok: number; cacheRead: number; cacheWrite: number };
  turns: TranscriptTurn[];
}

// ── Aggregate snapshot ─────────────────────────────────────────────────────

/** Everything the console loads on boot. Real deployments will fetch these
 *  collections lazily per screen; the mock returns them from one seed. */
export interface LoomSnapshot {
  now: string;
  keys: VirtualKey[];
  tenants: Tenant[];
  credOverrides: CredOverride[];
  providers: Provider[];
  stats: GatewayStats;
  spendByHour: number[];
  priorByHour: number[];
  usageDaily: UsageDaily;
  topModels: BarItem[];
  topKeys: BarItem[];
  events: GatewayEvent[];
  usageByKey: UsageByKey[];
  mcpServers: McpServer[];
  conversations: Conversation[];
}

// ── Request payloads ───────────────────────────────────────────────────────

export interface CreateKeyInput {
  name: string;
  tenant: string;
  scopes: string[];
  cap: number | null;
  window: BudgetWindow | null;
  mode: BudgetMode;
}

export type ConnectivityResult =
  | { ok: true; detail: string }
  | { ok: false; detail: string };
