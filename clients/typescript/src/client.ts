/**
 * A small fluent wrapper over Loom's HTTP API.
 *
 * The generated OpenAPI types (`./generated`) pin the request-body shapes; this
 * layer adds an ergonomic, chainable surface and typed domain results. The
 * request-shaping helpers ({@link ConversationBuilder.buildOptions} and the
 * turn bodies) are pure and unit-tested independently of any network.
 */

import { Transport, parseSse, LoomError, type TransportConfig } from "./http.js";
import type {
  Conversation,
  ContentPart,
  ConversationOptions,
  McpServerRef,
  Message,
  ServerTool,
  ToolDefinition,
  TurnEvent,
  UsageRollupResponse,
  WhoAmI,
} from "./types.js";

export { LoomError } from "./http.js";
export type * from "./types.js";

/** The default provider a conversation binds to when none is given. */
const DEFAULT_PROVIDER = "anthropic";

/** A user turn: either a plain string, ready-made content parts, or a Message. */
export type TurnInput = string | ContentPart[] | Message;

/** Normalises a {@link TurnInput} into the `content` array the API expects. */
export function toContent(input: TurnInput): ContentPart[] {
  if (typeof input === "string") {
    return [{ type: "text", text: input }];
  }
  if (Array.isArray(input)) {
    return input;
  }
  return input.content;
}

/** Options for opening a conversation. */
export interface ConversationInit {
  /** The model id, as the provider expects it (e.g. `claude-haiku-4-5-20251001`). */
  model: string;
  /** The provider to bind to. Defaults to `anthropic`. */
  provider?: string;
  /** An optional system prompt applied to the whole conversation. */
  system?: string;
  /** Free-form caller metadata (tags, correlation ids, …). */
  metadata?: unknown;
  /** Base request options applied to every turn (further chainable). */
  options?: ConversationOptions;
}

/** A stateless turn request (`POST /v1/turns`), nothing persisted. */
export interface StatelessTurnInit {
  model: string;
  provider?: string;
  system?: string;
  messages: Message[];
  options?: ConversationOptions;
}

/**
 * A lazily-created, tenant-scoped conversation with a fluent option surface.
 *
 * Option setters (`withMcp`, `cached`, `withServerTool`, …) mutate and return
 * `this`, so they chain. The remote conversation is created on the first
 * `send`/`stream`/`create` call and reused thereafter.
 */
export class ConversationBuilder {
  private readonly transport: Transport;
  private readonly init: ConversationInit;
  private options: ConversationOptions;
  private conversationId: string | undefined;
  private createPromise: Promise<Conversation> | undefined;

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
   * References an MCP server for every turn. A string is treated as a named
   * reference (the gateway resolves the URL + token server-side); pass a full
   * {@link McpServerRef} for the inline/advanced form. Repeated names are
   * de-duplicated (last wins).
   */
  withMcp(server: string | McpServerRef): this {
    const ref: McpServerRef =
      typeof server === "string" ? { name: server } : server;
    const existing = this.options.mcp_servers ?? [];
    const filtered = existing.filter((s) => s.name !== ref.name);
    this.options.mcp_servers = [...filtered, ref];
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
  withTools(...tools: ToolDefinition[]): this {
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
  async create(): Promise<Conversation> {
    if (this.createPromise) return this.createPromise;
    const body = {
      provider: this.init.provider ?? DEFAULT_PROVIDER,
      model: this.init.model,
      system: this.init.system,
      metadata: this.init.metadata,
    };
    this.createPromise = this.transport
      .json<Conversation>("POST", "/v1/conversations", body)
      .then(({ data }) => {
        this.conversationId = data.id;
        return data;
      });
    return this.createPromise;
  }

  /** Fetches the conversation with a page of its message history. */
  async fetch(page?: { limit?: number; offset?: number }): Promise<Conversation> {
    const id = await this.ensureId();
    const query = new URLSearchParams();
    if (page?.limit !== undefined) query.set("limit", String(page.limit));
    if (page?.offset !== undefined) query.set("offset", String(page.offset));
    const suffix = query.toString() ? `?${query}` : "";
    const { data } = await this.transport.json<Conversation>(
      "GET",
      `/v1/conversations/${id}${suffix}`,
    );
    return data;
  }

  /** Appends a user turn and returns the assistant {@link Message} (non-streaming). */
  async send(input: TurnInput): Promise<Message> {
    const id = await this.ensureId();
    const { data } = await this.transport.json<Message>(
      "POST",
      `/v1/conversations/${id}/turns`,
      { content: toContent(input), stream: false, options: this.buildOptions() },
    );
    return data;
  }

  /**
   * Appends a user turn and streams the assistant turn as {@link TurnEvent}s.
   * A native `error` SSE frame surfaces as a thrown {@link LoomError}.
   */
  async *stream(input: TurnInput): AsyncGenerator<TurnEvent, void, unknown> {
    const id = await this.ensureId();
    const response = await this.transport.openSse(
      "POST",
      `/v1/conversations/${id}/turns`,
      { content: toContent(input), stream: true, options: this.buildOptions() },
    );
    yield* streamTurnEvents(response);
  }

  private async ensureId(): Promise<string> {
    if (this.conversationId) return this.conversationId;
    await this.create();
    // `create` sets `conversationId`.
    return this.conversationId as string;
  }
}

/** Parses an open SSE response into normalised {@link TurnEvent}s. */
async function* streamTurnEvents(
  response: Response,
): AsyncGenerator<TurnEvent, void, unknown> {
  for await (const frame of parseSse(response)) {
    const parsed = JSON.parse(frame.data) as unknown;
    if (frame.event === "error") {
      const env = parsed as { error?: { message?: string } };
      throw new LoomError(200, env?.error?.message ?? "stream error", parsed);
    }
    yield parsed as TurnEvent;
  }
}

/** The top-level Loom client. */
export class LoomClient {
  private readonly transport: Transport;

  constructor(config: TransportConfig) {
    this.transport = new Transport(config);
  }

  /** Opens a fluent, lazily-created conversation builder. */
  conversation(init: ConversationInit): ConversationBuilder {
    return new ConversationBuilder(this.transport, init);
  }

  /** Fetches an existing conversation by id, with a page of its history. */
  async getConversation(
    id: string,
    page?: { limit?: number; offset?: number },
  ): Promise<Conversation> {
    const query = new URLSearchParams();
    if (page?.limit !== undefined) query.set("limit", String(page.limit));
    if (page?.offset !== undefined) query.set("offset", String(page.offset));
    const suffix = query.toString() ? `?${query}` : "";
    const { data } = await this.transport.json<Conversation>(
      "GET",
      `/v1/conversations/${id}${suffix}`,
    );
    return data;
  }

  /** Deletes a conversation by id. */
  async deleteConversation(id: string): Promise<void> {
    await this.transport.json<void>("DELETE", `/v1/conversations/${id}`);
  }

  /** Runs a stateless (non-persisted) turn, returning the assistant message. */
  async turn(init: StatelessTurnInit): Promise<Message> {
    const { data } = await this.transport.json<Message>("POST", "/v1/turns", {
      provider: init.provider ?? DEFAULT_PROVIDER,
      model: init.model,
      system: init.system,
      messages: init.messages,
      options: init.options,
      stream: false,
    });
    return data;
  }

  /** Runs a stateless turn as a stream of {@link TurnEvent}s. */
  async *streamTurn(
    init: StatelessTurnInit,
  ): AsyncGenerator<TurnEvent, void, unknown> {
    const response = await this.transport.openSse("POST", "/v1/turns", {
      provider: init.provider ?? DEFAULT_PROVIDER,
      model: init.model,
      system: init.system,
      messages: init.messages,
      options: init.options,
      stream: true,
    });
    yield* streamTurnEvents(response);
  }

  /** Fetches a tenant-scoped usage rollup. */
  async usage(params?: {
    from?: string;
    to?: string;
    group_by?: "key" | "model" | "conversation";
  }): Promise<UsageRollupResponse> {
    const query = new URLSearchParams();
    if (params?.from) query.set("from", params.from);
    if (params?.to) query.set("to", params.to);
    if (params?.group_by) query.set("group_by", params.group_by);
    const suffix = query.toString() ? `?${query}` : "";
    const { data } = await this.transport.json<UsageRollupResponse>(
      "GET",
      `/v1/usage${suffix}`,
    );
    return data;
  }

  /** Echoes the authenticated identity (`GET /v1/whoami`). */
  async whoami(): Promise<WhoAmI> {
    const { data } = await this.transport.json<WhoAmI>("GET", "/v1/whoami");
    return data;
  }
}

/** Creates a Loom client. */
export function createLoomClient(config: TransportConfig): LoomClient {
  return new LoomClient(config);
}
