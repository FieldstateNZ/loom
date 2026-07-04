/**
 * {@link Transport} — the low-level HTTP boundary shared by the fluent client.
 *
 * It is a small class (not a bag of functions) because it holds genuine
 * instance state: the resolved base URL, the tenant key, the `fetch` to use, and
 * an optional logger. It carries auth, decodes JSON responses against a Zod
 * schema, and turns every failure — transport, HTTP, or malformed body — into a
 * {@link LoomError} returned in a {@link Result}. It never throws for a failure
 * the gateway is expected to report.
 */

import type { z } from "zod";

import type { TransportConfig } from "./client-config.js";
import type { Logger } from "./logger.types.js";
import { decodeError, networkError, parseErrorEnvelope, zodIssues } from "./loom-error.js";
import type { LoomError } from "./loom-error.types.js";
import { err, ok } from "./result.js";
import type { Result } from "./result.types.js";

/** A thin, typed wrapper over `fetch` that carries auth and returns Results. */
export class Transport {
  private readonly baseUrl: string;
  private readonly apiKey: string;
  private readonly fetchImpl: typeof fetch;
  private readonly logger: Logger | undefined;

  /**
   * @param config - The validated client config. `baseUrl` has any trailing
   *   slashes stripped so path concatenation stays clean.
   */
  constructor(config: TransportConfig) {
    // Strip trailing slashes without a regex — a backtracking `/\/+$/` on a long
    // run of slashes is flagged as polynomial-ReDoS by static analysis.
    let base = config.baseUrl;
    while (base.endsWith("/")) base = base.slice(0, -1);
    this.baseUrl = base;
    this.apiKey = config.apiKey;
    this.fetchImpl = config.fetch ?? globalThis.fetch;
    this.logger = config.logger;
    if (!this.fetchImpl) {
      // No `fetch` in the runtime is an environment/programmer error, not a
      // gateway-reported failure, so it throws rather than returning a Result.
      throw new Error("no global fetch available; pass a fetch implementation");
    }
  }

  /** Builds the auth + content headers, merging any per-request extras. */
  private headers(extra?: Record<string, string>): Record<string, string> {
    return {
      authorization: `Bearer ${this.apiKey}`,
      "content-type": "application/json",
      ...extra,
    };
  }

  /** Reads a non-2xx response body and maps it to a structured HTTP error. */
  private async readError(response: Response): Promise<LoomError> {
    const text = await response.text().catch(() => "");
    this.logger?.warn?.(`loom: ${response.status} ${response.statusText}`, { path: response.url });
    return parseErrorEnvelope(response.status, response.statusText, text);
  }

  /**
   * Issues a request and decodes a JSON response against `schema`.
   *
   * @param schema - The Zod schema the response body must satisfy. Use
   *   `z.void()` for endpoints with no content (e.g. `DELETE`).
   * @param method - The HTTP method.
   * @param path - The path appended to the base URL (e.g. `/v1/whoami`).
   * @param body - An optional JSON request body.
   */
  async requestJson<S extends z.ZodTypeAny>(
    schema: S,
    method: string,
    path: string,
    body?: unknown,
  ): Promise<Result<z.infer<S>, LoomError>> {
    this.logger?.debug?.(`loom: ${method} ${path}`);
    let response: Response;
    try {
      response = await this.fetchImpl(`${this.baseUrl}${path}`, {
        method,
        headers: this.headers(),
        // Conditionally include `body` — under exactOptionalPropertyTypes a
        // `body: undefined` is rejected, and GET/DELETE carry no body.
        ...(body === undefined ? {} : { body: JSON.stringify(body) }),
      });
    } catch (cause) {
      return err(networkError(cause));
    }
    if (!response.ok) {
      return err(await this.readError(response));
    }
    const text = await response.text();
    let json: unknown;
    try {
      // Empty bodies (204 No Content) decode to `undefined`.
      json = text ? JSON.parse(text) : undefined;
    } catch {
      return err(decodeError("response body was not valid JSON"));
    }
    const parsed = schema.safeParse(json);
    if (!parsed.success) {
      return err(decodeError("response did not match the expected shape", zodIssues(parsed.error)));
    }
    return ok(parsed.data);
  }

  /**
   * Opens a `text/event-stream` request, returning the raw response for a
   * caller to parse, or a {@link LoomError} if the stream could not be opened.
   *
   * @param method - The HTTP method.
   * @param path - The path appended to the base URL.
   * @param body - An optional JSON request body.
   */
  async openSse(method: string, path: string, body?: unknown): Promise<Result<Response, LoomError>> {
    this.logger?.debug?.(`loom: ${method} ${path} (sse)`);
    let response: Response;
    try {
      response = await this.fetchImpl(`${this.baseUrl}${path}`, {
        method,
        headers: this.headers({ accept: "text/event-stream" }),
        ...(body === undefined ? {} : { body: JSON.stringify(body) }),
      });
    } catch (cause) {
      return err(networkError(cause));
    }
    if (!response.ok) {
      return err(await this.readError(response));
    }
    if (!response.body) {
      return err(decodeError("SSE response had no body"));
    }
    return ok(response);
  }
}
