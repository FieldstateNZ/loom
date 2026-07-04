/**
 * {@link LoomClient} — the top-level entry point to Loom's HTTP API.
 *
 * A thin class holding one piece of state — the {@link Transport} — and exposing
 * the endpoint methods plus a factory for the fluent {@link ConversationBuilder}.
 * Every network method returns a {@link Result}; construct it via
 * {@link createLoomClient} rather than directly, so the config is validated.
 */

import { z } from "zod";

import { ConversationBuilder } from "./conversation-builder.js";
import type { ConversationInit } from "./conversation-init.types.js";
import type { LoomError } from "./loom-error.types.js";
import { conversationSchema } from "./models/conversation.js";
import type { Conversation } from "./models/conversation.js";
import { mcpServerListResponseSchema } from "./models/mcp-server-list.js";
import type { Message } from "./models/message.js";
import type { TurnEvent } from "./models/turn-event.js";
import { usageRollupResponseSchema } from "./models/usage-rollup.js";
import type { UsageGroupBy, UsageRollupResponse } from "./models/usage-rollup.js";
import { whoAmISchema } from "./models/whoami.js";
import type { WhoAmI } from "./models/whoami.js";
import { pageQuery } from "./page-query.js";
import type { PageParams } from "./page-query.types.js";
import { ok } from "./result.js";
import type { Result } from "./result.types.js";
import { runStatelessTurn, streamStatelessTurn } from "./stateless-turn.js";
import type { StatelessTurnInit } from "./stateless-turn.types.js";
import type { Transport } from "./transport.js";

/** Filters for a usage rollup: an optional time window and grouping dimension. */
interface UsageParams {
  /** Inclusive lower bound (RFC 3339), or omitted for no lower bound. */
  readonly from?: string;
  /** Exclusive upper bound (RFC 3339), or omitted for no upper bound. */
  readonly to?: string;
  /** The dimension to group rows by. */
  readonly group_by?: UsageGroupBy;
}

/** The top-level Loom client. */
export class LoomClient {
  private readonly transport: Transport;

  /** @param transport - A configured transport (see {@link createLoomClient}). */
  constructor(transport: Transport) {
    this.transport = transport;
  }

  /** Opens a fluent, lazily-created conversation builder. */
  conversation(init: ConversationInit): ConversationBuilder {
    return new ConversationBuilder(this.transport, init);
  }

  /** Fetches an existing conversation by id, with a page of its history. */
  async getConversation(id: string, page?: PageParams): Promise<Result<Conversation, LoomError>> {
    return this.transport.requestJson(
      conversationSchema,
      "GET",
      `/v1/conversations/${id}${pageQuery(page)}`,
    );
  }

  /** Deletes a conversation by id. */
  async deleteConversation(id: string): Promise<Result<void, LoomError>> {
    return this.transport.requestJson(z.void(), "DELETE", `/v1/conversations/${id}`);
  }

  /** Runs a stateless (non-persisted) turn, returning the assistant message. */
  async turn(init: StatelessTurnInit): Promise<Result<Message, LoomError>> {
    return runStatelessTurn(this.transport, init);
  }

  /** Runs a stateless turn as a stream of {@link TurnEvent}s. */
  streamTurn(init: StatelessTurnInit): AsyncGenerator<Result<TurnEvent, LoomError>, void, unknown> {
    return streamStatelessTurn(this.transport, init);
  }

  /** Fetches a tenant-scoped usage rollup. */
  async usage(params?: UsageParams): Promise<Result<UsageRollupResponse, LoomError>> {
    const query = new URLSearchParams();
    if (params?.from) query.set("from", params.from);
    if (params?.to) query.set("to", params.to);
    if (params?.group_by) query.set("group_by", params.group_by);
    const rendered = query.toString();
    return this.transport.requestJson(
      usageRollupResponseSchema,
      "GET",
      `/v1/usage${rendered ? `?${rendered}` : ""}`,
    );
  }

  /** Echoes the authenticated identity (`GET /v1/whoami`). */
  async whoami(): Promise<Result<WhoAmI, LoomError>> {
    return this.transport.requestJson(whoAmISchema, "GET", "/v1/whoami");
  }

  /**
   * Lists the names of MCP servers registered for the caller's tenant
   * (`GET /v1/mcp-servers`), so a `withMcp(name)` reference can be validated
   * before use. Never carries a URL or authorization token — those stay
   * server-side.
   */
  async mcpServers(): Promise<Result<readonly string[], LoomError>> {
    const result = await this.transport.requestJson(
      mcpServerListResponseSchema,
      "GET",
      "/v1/mcp-servers",
    );
    if (!result.ok) return result;
    return ok(result.value.servers);
  }
}
