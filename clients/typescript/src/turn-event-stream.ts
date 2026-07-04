/**
 * Decodes an open SSE turn response into validated {@link TurnEvent}s.
 *
 * Bridges the byte-level {@link parseSse} and the domain: each frame's JSON is
 * validated against {@link turnEventSchema}, and each yielded item is a
 * {@link Result} so a mid-stream failure (a terminal `error` frame, or a
 * malformed frame) surfaces in-band rather than as a thrown exception.
 */

import { decodeError, streamError, zodIssues } from "./loom-error.js";
import type { LoomError } from "./loom-error.types.js";
import { turnEventSchema } from "./models/turn-event.js";
import type { TurnEvent } from "./models/turn-event.js";
import { err, ok } from "./result.js";
import type { Result } from "./result.types.js";
import { parseSse } from "./sse-parser.js";

/**
 * Iterates an open `text/event-stream` response, yielding one {@link Result}
 * per frame. A terminal `error` frame is yielded as a failure and ends the
 * stream; any other frame is validated and yielded as a success or a decode
 * failure.
 *
 * @param response - The open SSE response returned by the transport.
 */
export async function* streamTurnEvents(
  response: Response,
): AsyncGenerator<Result<TurnEvent, LoomError>, void, unknown> {
  for await (const frame of parseSse(response)) {
    let payload: unknown;
    try {
      payload = JSON.parse(frame.data);
    } catch {
      yield err(decodeError("SSE frame was not valid JSON"));
      continue;
    }
    if (frame.event === "error") {
      yield err(streamError(payload));
      return;
    }
    const parsed = turnEventSchema.safeParse(payload);
    if (!parsed.success) {
      yield err(decodeError("stream event did not match the expected shape", zodIssues(parsed.error)));
      continue;
    }
    yield ok(parsed.data);
  }
}
