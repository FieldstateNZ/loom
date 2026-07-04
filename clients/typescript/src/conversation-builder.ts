/**
 * {@link ConversationBuilder} — a fluent, lazily-created conversation.
 *
 * This is a legitimate class: it accumulates request options across chained
 * setter calls and holds lazy instance state (the remote conversation id, and
 * the in-flight create so concurrent turns share one round-trip). Option setters
 * mutate a private draft and return `this`, so they chain; the remote
 * conversation is created on the first `send`/`stream`/`create`/`fetch` and
 * reused thereafter. Every network method returns a {@link Result}.
 */

import type { ConversationInit } from "./conversation-init.types.js";
import { DEFAULT_PROVIDER } from "./defaults.js";
import type { LoomError } from "./loom-error.types.js";
import { conversationSchema } from "./models/conversation.js";
import { turnResponseSchema } from "./models/turn-response.js";
import type { Conversation, ConversationOptions, McpServerRef } from "./models/index.js";
import type { ServerTool, ToolDefinition, TurnEvent, TurnResponse } from "./models/index.js";
import { pageQuery } from "./page-query.js";
import type { PageParams } from "./page-query.types.js";
import { ok } from "./result.js";
import type { Result } from "./result.types.js";
import { toContent } from "./to-content.js";
import type { TurnInput } from "./to-content.types.js";
import type { Transport } from "./transport.js";
import { streamTurnEvents } from "./turn-event-stream.js";

/** A writable draft of {@link ConversationOptions} the builder accumulates into. */
type MutableOptions = { -readonly [K in keyof ConversationOptions]: ConversationOptions[K] };

/** A fluent, lazily-created, tenant-scoped conversation. */
export class ConversationBuilder {
  private readonly transport: Transport;
  private readonly init: ConversationInit;
  private options: MutableOptions;
  private conversationId: string | undefined;
  private createPromise: Promise<Result<Conversation, LoomError>> | undefined;

  /**
   * @param transport - The HTTP transport to issue requests through.
   * @param init - The conversation binding and base options.
   */
  constructor(transport: Transport, init: ConversationInit) {
    this.transport = transport;
    this.init = init;
    // Defensive clone so a caller's shared options object is never mutated.
    this.options = structuredClone(init.options ?? {});
  }

  /** The remote conversation id once created, else `undefined`. */
  get id(): string | undefined {
    return this.conversationId;
  }

  /** A read-only snapshot of the accumulated request options. */
  buildOptions(): ConversationOptions {
    return structuredClone(this.options);
  }

  /**
   * References an MCP server for every turn. A string is a named reference (the
   * gateway resolves the URL + token server-side); a full {@link McpServerRef}
   * is the inline form. Repeated names are de-duplicated (last wins).
   */
  withMcp(server: string | McpServerRef): this {
    const ref: McpServerRef = typeof server === "string" ? { name: server } : server;
    const existing = this.options.mcp_servers ?? [];
    this.options.mcp_servers = [...existing.filter((s) => s.name !== ref.name), ref];
    return this;
  }

  /** Enables Loom's automatic prompt caching (`auto_cache`) for every turn. */
  cached(): this {
    return this.withCache(true);
  }

  /** Sets `auto_cache` explicitly. */
  withCache(enabled = true): this {
    this.options.auto_cache = enabled;
    return this;
  }

  /** Offers a provider-executed (server-side) tool for every turn. */
  withServerTool(tool: ServerTool): this {
    this.options.server_tools = [...(this.options.server_tools ?? []), tool];
    return this;
  }

  /** Offers one or more client-side tool definitions. */
  withTools(...tools: readonly ToolDefinition[]): this {
    this.options.tools = [...(this.options.tools ?? []), ...tools];
    return this;
  }

  /** Sets the sampling temperature. */
  temperature(value: number): this {
    this.options.temperature = value;
    return this;
  }

  /** Sets the maximum number of tokens the model may generate. */
  maxTokens(value: number): this {
    this.options.max_tokens = value;
    return this;
  }

  /** Merges a partial options patch over the accumulated options. */
  withOptions(patch: ConversationOptions): this {
    this.options = { ...this.options, ...patch };
    return this;
  }

  /** Creates the remote conversation now (idempotent), returning it. */
  async create(): Promise<Result<Conversation, LoomError>> {
    if (!this.createPromise) {
      const body = {
        provider: this.init.provider ?? DEFAULT_PROVIDER,
        model: this.init.model,
        system: this.init.system,
        metadata: this.init.metadata,
      };
      this.createPromise = this.transport
        .requestJson(conversationSchema, "POST", "/v1/conversations", body)
        .then((result) => {
          if (result.ok) this.conversationId = result.value.id;
          // On failure, clear the memo so a later call can retry.
          else this.createPromise = undefined;
          return result;
        });
    }
    return this.createPromise;
  }

  /** Fetches the conversation with a page of its message history. */
  async fetch(page?: PageParams): Promise<Result<Conversation, LoomError>> {
    const id = await this.ensureId();
    if (!id.ok) return id;
    const path = `/v1/conversations/${id.value}${pageQuery(page)}`;
    return this.transport.requestJson(conversationSchema, "GET", path);
  }

  /**
   * Appends a user turn and returns the assistant {@link TurnResponse} —
   * `{ message, cost }` (non-streaming). `cost` is Loom's authoritative priced
   * cost for the turn; `null` when no price is configured for the
   * (provider, model).
   */
  async send(input: TurnInput): Promise<Result<TurnResponse, LoomError>> {
    const id = await this.ensureId();
    if (!id.ok) return id;
    const path = `/v1/conversations/${id.value}/turns`;
    const body = { content: toContent(input), stream: false, options: this.buildOptions() };
    return this.transport.requestJson(turnResponseSchema, "POST", path, body);
  }

  /** Appends a user turn and streams the assistant turn as {@link TurnEvent}s. */
  async *stream(input: TurnInput): AsyncGenerator<Result<TurnEvent, LoomError>, void, unknown> {
    const id = await this.ensureId();
    if (!id.ok) {
      yield id;
      return;
    }
    const path = `/v1/conversations/${id.value}/turns`;
    const body = { content: toContent(input), stream: true, options: this.buildOptions() };
    const opened = await this.transport.openSse("POST", path, body);
    if (!opened.ok) {
      yield opened;
      return;
    }
    yield* streamTurnEvents(opened.value);
  }

  /** Resolves the conversation id, creating the conversation if needed. */
  private async ensureId(): Promise<Result<string, LoomError>> {
    if (this.conversationId) return ok(this.conversationId);
    const created = await this.create();
    return created.ok ? ok(created.value.id) : created;
  }
}
