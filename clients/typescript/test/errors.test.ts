/**
 * Unit tests for the {@link LoomError} taxonomy — the envelope-to-error
 * decoding done by {@link Transport} (`readError`/`requestJson`) and exercised
 * here through the public {@link LoomClient} surface, with a fake `fetch`
 * injected via {@link createLoomClient} (the same DI pattern `shaping.test.ts`
 * uses). Each case asserts the discriminated `kind` and, for HTTP failures,
 * that the gateway's `code`/`details` survive verbatim — including a code the
 * client's `LoomErrorCode` enum does not know about, which must still pass
 * through untouched because the type is deliberately open-ended.
 */

import { test } from "node:test";
import assert from "node:assert/strict";

import { createLoomClient } from "../src/index.ts";
import type { LoomError, Result } from "../src/index.ts";

/** Builds a client whose `fetch` always answers with `response`, regardless of request. */
function clientReturning(response: () => Response | Promise<Response>) {
  const fetchImpl: typeof fetch = async () => response();
  const created = createLoomClient({
    baseUrl: "http://loom.test",
    apiKey: "loom_test_key",
    fetch: fetchImpl,
  });
  assert.ok(created.ok, "client config is valid");
  return created.value;
}

/** A gateway error envelope response for a given status/code/extras. */
function envelopeResponse(
  status: number,
  statusText: string,
  code: string,
  extra?: { readonly details?: unknown },
): Response {
  return new Response(
    JSON.stringify({ error: { code, message: `${code} for real`, ...extra } }),
    { status, statusText, headers: { "content-type": "application/json" } },
  );
}

/** Asserts `result` is a failed {@link Result} and returns its {@link LoomError}. */
function assertErr<T>(result: Result<T, LoomError>): LoomError {
  assert.ok(!result.ok, "expected a failed Result");
  return result.error;
}

test("HTTP 402 budget_exceeded decodes with status, code, and details preserved", async () => {
  const loom = clientReturning(() =>
    envelopeResponse(402, "Payment Required", "budget_exceeded", {
      details: { limit_usd: 100, spent_usd: 101.5 },
    }),
  );
  const error = assertErr(await loom.whoami());
  assert.equal(error.kind, "http");
  if (error.kind !== "http") return;
  assert.equal(error.status, 402);
  assert.equal(error.code, "budget_exceeded");
  assert.equal(error.message, "budget_exceeded for real");
  assert.deepEqual(error.details, { limit_usd: 100, spent_usd: 101.5 });
});

test("HTTP 422 capability_unsupported decodes with the matching code", async () => {
  const loom = clientReturning(() =>
    envelopeResponse(422, "Unprocessable Entity", "capability_unsupported"),
  );
  const error = assertErr(await loom.whoami());
  assert.equal(error.kind, "http");
  if (error.kind !== "http") return;
  assert.equal(error.status, 422);
  assert.equal(error.code, "capability_unsupported");
});

test("HTTP 404 with a code unknown to LoomErrorCode still surfaces it verbatim", async () => {
  const loom = clientReturning(() =>
    envelopeResponse(404, "Not Found", "mcp_server_not_configured"),
  );
  const error = assertErr(await loom.whoami());
  assert.equal(error.kind, "http");
  if (error.kind !== "http") return;
  assert.equal(error.status, 404);
  // Not one of LoomErrorCode's known literals, but the type is deliberately
  // open-ended (`| (string & {})`), so the server's code passes through as-is.
  assert.equal(error.code, "mcp_server_not_configured");
});

test("HTTP 429 rate_limited decodes with the matching code", async () => {
  const loom = clientReturning(() => envelopeResponse(429, "Too Many Requests", "rate_limited"));
  const error = assertErr(await loom.whoami());
  assert.equal(error.kind, "http");
  if (error.kind !== "http") return;
  assert.equal(error.status, 429);
  assert.equal(error.code, "rate_limited");
});

test("a fetch rejection decodes to a network error", async () => {
  const fetchImpl: typeof fetch = async () => {
    throw new Error("getaddrinfo ENOTFOUND loom.test");
  };
  const created = createLoomClient({
    baseUrl: "http://loom.test",
    apiKey: "loom_test_key",
    fetch: fetchImpl,
  });
  assert.ok(created.ok, "client config is valid");
  const error = assertErr(await created.value.whoami());
  assert.equal(error.kind, "network");
  if (error.kind !== "network") return;
  assert.match(error.message, /ENOTFOUND/);
});

test("a non-JSON 2xx body decodes to a decode error", async () => {
  const loom = clientReturning(
    () => new Response("<html>not json</html>", { status: 200, headers: { "content-type": "text/html" } }),
  );
  const error = assertErr(await loom.whoami());
  assert.equal(error.kind, "decode");
  if (error.kind !== "decode") return;
  assert.match(error.message, /not valid JSON/);
});

test("a 2xx body that does not match the response schema decodes to a decode error", async () => {
  // Valid JSON, but missing every field `whoAmISchema` requires.
  const loom = clientReturning(
    () => new Response(JSON.stringify({ unexpected: true }), {
      status: 200,
      headers: { "content-type": "application/json" },
    }),
  );
  const error = assertErr(await loom.whoami());
  assert.equal(error.kind, "decode");
  if (error.kind !== "decode") return;
  assert.match(error.message, /did not match the expected shape/);
  assert.ok(error.issues && error.issues.length > 0, "zod issues were carried through");
});
