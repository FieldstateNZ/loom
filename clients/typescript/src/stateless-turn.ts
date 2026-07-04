/**
 * The stateless-turn helpers (`POST /v1/turns`): a one-shot turn where the
 * caller supplies the whole history and nothing is persisted.
 *
 * Kept as free functions (not methods) because they hold no state — they only
 * shape a request body and hand it to the transport. The body-shaping is pure
 * and shared by the non-streaming and streaming variants.
 */

import { DEFAULT_PROVIDER } from "./defaults.js";
import type { LoomError } from "./loom-error.types.js";
import { turnResponseSchema } from "./models/turn-response.js";
import type { TurnResponse } from "./models/turn-response.js";
import type { TurnEvent } from "./models/turn-event.js";
import type { Result } from "./result.types.js";
import type { StatelessTurnInit } from "./stateless-turn.types.js";
import type { Transport } from "./transport.js";
import { streamTurnEvents } from "./turn-event-stream.js";

/** Shapes the `/v1/turns` request body from a {@link StatelessTurnInit}. */
function statelessBody(init: StatelessTurnInit, stream: boolean): unknown {
  return {
    provider: init.provider ?? DEFAULT_PROVIDER,
    model: init.model,
    system: init.system,
    messages: init.messages,
    options: init.options,
    stream,
  };
}

/**
 * Runs a stateless (non-persisted) turn, returning the assistant
 * {@link TurnResponse} — `{ message, cost }`, `cost` being Loom's
 * authoritative priced cost for the turn.
 *
 * @param transport - The HTTP transport to issue the request through.
 * @param init - The model binding, message history, and options.
 */
export function runStatelessTurn(
  transport: Transport,
  init: StatelessTurnInit,
): Promise<Result<TurnResponse, LoomError>> {
  return transport.requestJson(
    turnResponseSchema,
    "POST",
    "/v1/turns",
    statelessBody(init, false),
  );
}

/**
 * Runs a stateless turn as a stream of {@link TurnEvent}s.
 *
 * @param transport - The HTTP transport to issue the request through.
 * @param init - The model binding, message history, and options.
 */
export async function* streamStatelessTurn(
  transport: Transport,
  init: StatelessTurnInit,
): AsyncGenerator<Result<TurnEvent, LoomError>, void, unknown> {
  const opened = await transport.openSse("POST", "/v1/turns", statelessBody(init, true));
  if (!opened.ok) {
    yield opened;
    return;
  }
  yield* streamTurnEvents(opened.value);
}
