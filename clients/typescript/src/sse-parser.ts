/**
 * A Server-Sent Events byte-stream parser.
 *
 * Turns a streaming HTTP body into an async iterator of `{ event, data }`
 * frames, honouring multi-line `data:` fields, `:` comment/keep-alive lines,
 * and blank-line frame boundaries. Named events (`event: error`) keep their name
 * so a consumer can tell an error frame from a data frame. Terminal
 * `data: [DONE]` sentinels are skipped. This layer is transport-only: it does
 * not parse JSON or decide what a frame means.
 */

/** One decoded SSE frame: the event name and its (possibly multi-line) data. */
export interface SseFrame {
  /** The `event:` field, or `"message"` when none was sent. */
  readonly event: string;
  /** The joined `data:` field(s), with the trailing newline stripped. */
  readonly data: string;
}

/** Splits one SSE block (the text between blank lines) into at most one frame. */
function* frameFromBlock(block: string): Generator<SseFrame> {
  let event = "message";
  const dataLines: string[] = [];
  for (const rawLine of block.split("\n")) {
    const line = rawLine.replace(/\r$/, "");
    if (line.startsWith(":")) continue; // comment / keep-alive
    const colon = line.indexOf(":");
    const field = colon === -1 ? line : line.slice(0, colon);
    let value = colon === -1 ? "" : line.slice(colon + 1);
    if (value.startsWith(" ")) value = value.slice(1);
    if (field === "event") event = value;
    else if (field === "data") dataLines.push(value);
  }
  if (dataLines.length > 0) {
    const data = dataLines.join("\n");
    if (data !== "[DONE]") yield { event, data };
  }
}

/**
 * Parses an SSE response body into an async iterator of {@link SseFrame}s.
 *
 * @param response - A `fetch` response whose body is a `text/event-stream`.
 */
export async function* parseSse(response: Response): AsyncGenerator<SseFrame, void, unknown> {
  const body = response.body;
  if (!body) return;
  const reader = body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      let sep: number;
      // Frames are separated by a blank line (\n\n).
      while ((sep = buffer.indexOf("\n\n")) !== -1) {
        const block = buffer.slice(0, sep);
        buffer = buffer.slice(sep + 2);
        yield* frameFromBlock(block);
      }
    }
    // Emit any trailing frame not terminated by a blank line.
    if (buffer.trim().length > 0) yield* frameFromBlock(buffer);
  } finally {
    reader.releaseLock();
  }
}
