/** Low-level HTTP + SSE plumbing shared by the fluent client. */

/** A structured error thrown when Loom returns a non-2xx response. */
export class LoomError extends Error {
  readonly status: number;
  readonly body: unknown;
  constructor(status: number, message: string, body: unknown) {
    super(message);
    this.name = "LoomError";
    this.status = status;
    this.body = body;
  }
}

/** Configuration for the low-level transport. */
export interface TransportConfig {
  /** Loom base URL, e.g. `http://127.0.0.1:8080`. No trailing slash required. */
  baseUrl: string;
  /** A tenant virtual key (`loom_...`), sent as `Authorization: Bearer`. */
  apiKey: string;
  /** Optional `fetch` override (for tests / non-browser runtimes). */
  fetch?: typeof fetch;
}

/** A thin, typed wrapper over `fetch` that carries auth and unwraps errors. */
export class Transport {
  private readonly baseUrl: string;
  private readonly apiKey: string;
  private readonly fetchImpl: typeof fetch;

  constructor(config: TransportConfig) {
    // Strip trailing slashes without a regex — a backtracking `/\/+$/` on a
    // long run of slashes is flagged as polynomial-ReDoS by static analysis.
    let base = config.baseUrl;
    while (base.endsWith("/")) base = base.slice(0, -1);
    this.baseUrl = base;
    this.apiKey = config.apiKey;
    this.fetchImpl = config.fetch ?? globalThis.fetch;
    if (!this.fetchImpl) {
      throw new Error("no global fetch available; pass a fetch implementation");
    }
  }

  private headers(extra?: Record<string, string>): Record<string, string> {
    return {
      authorization: `Bearer ${this.apiKey}`,
      "content-type": "application/json",
      ...extra,
    };
  }

  /** Issues a request and parses a JSON response, throwing {@link LoomError}. */
  async json<T>(
    method: string,
    path: string,
    body?: unknown,
  ): Promise<{ data: T; response: Response }> {
    const response = await this.fetchImpl(`${this.baseUrl}${path}`, {
      method,
      headers: this.headers(),
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    if (!response.ok) {
      throw await this.error(response);
    }
    // 204 No Content and other empty bodies parse to `undefined`.
    const text = await response.text();
    const data = (text ? JSON.parse(text) : undefined) as T;
    return { data, response };
  }

  /** Opens a `text/event-stream` request, returning the raw response. */
  async openSse(method: string, path: string, body?: unknown): Promise<Response> {
    const response = await this.fetchImpl(`${this.baseUrl}${path}`, {
      method,
      headers: this.headers({ accept: "text/event-stream" }),
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    if (!response.ok) {
      throw await this.error(response);
    }
    if (!response.body) {
      throw new LoomError(response.status, "SSE response had no body", null);
    }
    return response;
  }

  private async error(response: Response): Promise<LoomError> {
    let body: unknown = null;
    let message = `${response.status} ${response.statusText}`;
    try {
      const text = await response.text();
      if (text) {
        try {
          body = JSON.parse(text);
          const asObj = body as { error?: { message?: string }; message?: string };
          message = asObj?.error?.message ?? asObj?.message ?? message;
        } catch {
          body = text;
          message = text;
        }
      }
    } catch {
      /* ignore */
    }
    return new LoomError(response.status, message, body);
  }
}

/**
 * Parses a Server-Sent Events byte stream into an async iterator of raw event
 * payloads (the string after `data:`), honouring multi-line `data:` fields and
 * blank-line frame boundaries. Terminal `data: [DONE]` sentinels (if any) are
 * skipped. Named events (`event: error`) are surfaced with their name so a
 * consumer can distinguish an error frame from a data frame.
 */
export async function* parseSse(
  response: Response,
): AsyncGenerator<{ event: string; data: string }, void, unknown> {
  const body = response.body;
  if (!body) return;
  const reader = body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";

  const flush = function* (
    block: string,
  ): Generator<{ event: string; data: string }> {
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
  };

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
        yield* flush(block);
      }
    }
    // Emit any trailing frame not terminated by a blank line.
    if (buffer.trim().length > 0) yield* flush(buffer);
  } finally {
    reader.releaseLock();
  }
}
