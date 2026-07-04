/**
 * {@link collect} — reassembles the final assistant message, usage snapshot,
 * and stop reason from a streamed turn.
 *
 * Mirrors the server-side `TurnAccumulator` fold
 * (`crates/loom-server/src/v1/runner/stream.rs`) exactly: only *complete*
 * content parts (`content_part_complete` events, folded in ascending `index`
 * order) contribute to the final message — a bare `content_part_started` or
 * `content_part_delta` is ignored, the same way the gateway ignores them when
 * it reassembles the message it persists. `usage` is taken from a bare `usage`
 * event and then superseded by the `usage` carried on `turn_ended`, when
 * present; `stopReason` comes from `turn_ended.stop_reason`. Because the fold
 * is identical, a caller who drains a whole stream through {@link collect}
 * gets the same {@link Message} `client.turn()` would return for the same
 * input.
 *
 * A mid-stream failure — any yielded {@link Result} with `ok: false`, e.g. a
 * decoded {@link LoomStreamError} — short-circuits the fold and is returned
 * as-is. A stream that ends without ever yielding a `turn_ended` event (so no
 * `stopReason` was ever reported) is treated as a broken contract and reported
 * as a {@link LoomDecodeError}, rather than fabricating a stop reason.
 */

import type { CollectedTurn } from "./collect.types.js";
import { decodeError } from "./loom-error.js";
import type { LoomError } from "./loom-error.types.js";
import type { ContentPart } from "./models/content-part.js";
import type { TurnEvent, TurnEventKind } from "./models/turn-event.js";
import type { Usage } from "./models/usage.js";
import { err, ok } from "./result.js";
import type { Result } from "./result.types.js";

/**
 * Drains a stream of {@link Result}-wrapped {@link TurnEvent}s — such as the
 * generator returned by `client.streamTurn(init)` — and reassembles the
 * turn's final message, usage, and stop reason.
 *
 * @param events - The turn event stream to drain.
 */
export async function collect(
  events: AsyncIterable<Result<TurnEvent, LoomError>>,
): Promise<Result<CollectedTurn, LoomError>> {
  const parts = new Map<number, ContentPart>();
  let usage: Usage | undefined;
  let stopReason: string | undefined;

  for await (const event of events) {
    if (!event.ok) return event;
    const { kind } = event.value;
    switch (kind.type) {
      case "content_part_complete":
        parts.set(kind.index, kind.part);
        break;
      case "usage":
        usage = usageFromEvent(kind);
        break;
      case "turn_ended":
        stopReason = kind.stop_reason;
        if (kind.usage) usage = kind.usage;
        break;
      default:
        break;
    }
  }

  if (stopReason === undefined) {
    return err(decodeError("turn stream ended without a turn_ended event"));
  }

  const content = [...parts.entries()].sort(([a], [b]) => a - b).map(([, part]) => part);

  return ok({
    message: { role: "assistant", content, usage: usage ?? {} },
    usage: usage ?? {},
    stopReason,
  });
}

/**
 * Strips the `type` discriminant off a decoded `usage` event, leaving a plain
 * {@link Usage}. Built field-by-field (rather than a rest-destructure) because
 * `exactOptionalPropertyTypes` rejects an optional property explicitly set to
 * `undefined` — each field is only included when the event actually reported it.
 */
function usageFromEvent(kind: Extract<TurnEventKind, { type: "usage" }>): Usage {
  return {
    ...(kind.input_tokens !== undefined ? { input_tokens: kind.input_tokens } : {}),
    ...(kind.output_tokens !== undefined ? { output_tokens: kind.output_tokens } : {}),
    ...(kind.cache_read_tokens !== undefined ? { cache_read_tokens: kind.cache_read_tokens } : {}),
    ...(kind.cache_write_tokens !== undefined
      ? { cache_write_tokens: kind.cache_write_tokens }
      : {}),
    ...(kind.server_tool_use !== undefined ? { server_tool_use: kind.server_tool_use } : {}),
    ...(kind.raw !== undefined ? { raw: kind.raw } : {}),
  };
}
