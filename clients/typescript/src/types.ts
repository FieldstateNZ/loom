/**
 * Hand-authored domain types mirroring Loom's core serde shapes.
 *
 * Loom's OpenAPI document types most request bodies precisely but renders the
 * rich, `#[non_exhaustive]` domain enums (`ContentPart`, `TurnEvent`, …) as
 * opaque `Object`s — utoipa cannot express Rust's internally-tagged enums
 * losslessly. These types restore the real wire shapes so the fluent client can
 * offer a typed surface. They are kept deliberately faithful to
 * `crates/loom-core` and `crates/loom-provider`.
 */

/** Who authored a {@link Message}. */
export type Role = "user" | "assistant" | "provider";

/** The origin of an image or document's bytes. */
export type MediaSource =
  | { type: "base64"; media_type: string; data: string }
  | { type: "url"; url: string };

/** A provider-native citation payload, preserved verbatim. */
export type Citation = unknown;

/** How long a provider should keep a cache entry alive. */
export type CacheTtl = "five_minutes" | "one_hour";

/** A provider-agnostic prompt-cache breakpoint. */
export interface CacheHint {
  ttl?: CacheTtl;
}

/** A single, typed piece of message content (internally tagged on `type`). */
export type ContentPart =
  | { type: "text"; text: string; citations?: Citation[]; cache?: CacheHint }
  | { type: "image"; source: MediaSource; cache?: CacheHint }
  | { type: "document"; source: MediaSource; cache?: CacheHint }
  | {
      type: "tool_use";
      id: string;
      name: string;
      input: unknown;
      cache?: CacheHint;
    }
  | {
      type: "tool_result";
      tool_use_id: string;
      content: unknown;
      is_error?: boolean;
      cache?: CacheHint;
    }
  | { type: "server_tool_use"; id: string; name: string; input: unknown }
  | { type: "server_tool_result"; tool_use_id: string; content: unknown }
  | {
      type: "thinking";
      thinking: string;
      signature?: string;
      cache?: CacheHint;
    }
  | { type: "redacted_thinking"; data: string }
  | {
      type: "provider_extension";
      provider: string;
      kind: string;
      payload: unknown;
    };

/** A snapshot of the resource usage a provider reported for a response. */
export interface Usage {
  input_tokens?: number;
  output_tokens?: number;
  cache_read_tokens?: number;
  cache_write_tokens?: number;
  server_tool_use?: Record<string, number>;
  raw?: unknown;
}

/** A single turn in a conversation. */
export interface Message {
  role: Role;
  content: ContentPart[];
  usage?: Usage;
  raw?: unknown;
}

/** How a provider should treat cache hints on a non-caching model. */
export type CacheNegotiation = "soft_ignore" | "hard_fail";

/** A reference to an external MCP server the model may use. */
export interface McpServerRef {
  name: string;
  url?: string;
  authorization?: string;
  tool_configuration?: unknown;
}

/** A provider-executed (server-side) tool (internally tagged on `kind`). */
export type ServerTool =
  | {
      kind: "web_search";
      max_uses?: number;
      allowed_domains?: string[];
      blocked_domains?: string[];
    }
  | { kind: "code_execution" }
  | ({ kind: "raw" } & Record<string, unknown>);

/** The definition of a client-side tool the model may call. */
export interface ToolDefinition {
  name: string;
  description?: string;
  input_schema: unknown;
  cache?: CacheHint;
}

/** Options that shape how a provider should generate a response. */
export interface ConversationOptions {
  temperature?: number;
  max_tokens?: number;
  stop_sequences?: string[];
  tools?: ToolDefinition[];
  server_tools?: ServerTool[];
  auto_cache?: boolean;
  cache_negotiation?: CacheNegotiation;
  mcp_servers?: McpServerRef[];
  provider_options?: Record<string, unknown>;
}

/** The reason a provider stopped generating a turn. */
export type StopReason =
  | "end_turn"
  | "max_tokens"
  | "stop_sequence"
  | "tool_use"
  | "pause_turn"
  | "refusal"
  | string;

/** An incremental change to a streaming content part (tagged on `type`). */
export type ContentDelta =
  | { type: "text"; text: string }
  | { type: "json"; partial_json: string }
  | { type: "thinking"; thinking: string }
  | { type: "signature_delta"; signature: string }
  | { type: "citation"; citation: Citation };

/** The normalised, provider-agnostic classification of a {@link TurnEvent}. */
export type TurnEventKind =
  | { type: "turn_started" }
  | { type: "content_part_started"; index: number; part: ContentPart }
  | { type: "content_part_delta"; index: number; delta: ContentDelta }
  | { type: "content_part_complete"; index: number; part: ContentPart }
  | ({ type: "usage" } & Usage)
  | { type: "turn_ended"; stop_reason: StopReason; usage?: Usage }
  | { type: "other"; native_type?: string };

/**
 * A single streaming event: a normalised envelope plus the verbatim native
 * provider event it was derived from. Each SSE `data:` frame is one of these.
 */
export interface TurnEvent {
  kind: TurnEventKind;
  raw: unknown;
}

/** A tenant-scoped conversation, as returned by create/fetch. */
export interface Conversation {
  id: string;
  tenant_id: string;
  binding: { provider: string; model: string };
  system?: string | null;
  system_cache?: CacheHint | null;
  messages: Message[];
  metadata?: unknown;
  created_at?: string;
  updated_at?: string;
}

/** One grouped row in a usage-rollup response. */
export interface UsageRollupRow {
  group: string | null;
  event_count: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  cost: string;
  batch_cost: string;
  interactive_cost: string;
}

/** A usage-rollup response envelope. */
export interface UsageRollupResponse {
  group_by: "key" | "model" | "conversation";
  from: string | null;
  to: string | null;
  rows: UsageRollupRow[];
}

/** The authenticated identity echoed by `GET /v1/whoami`. */
export interface WhoAmI {
  tenant_id: string;
  key_id: string;
  key_prefix: string;
  scopes: string[];
}
