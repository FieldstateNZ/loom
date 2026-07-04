/**
 * Unit tests for {@link LoomClient.mcpServers} — `GET /v1/mcp-servers`. Uses
 * the same fake-`fetch` DI pattern as `errors.test.ts`: a canned `Response` is
 * returned regardless of the request, and the test asserts on the decoded
 * {@link Result}.
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

/** Asserts `result` is a failed {@link Result} and returns its {@link LoomError}. */
function assertErr<T>(result: Result<T, LoomError>): LoomError {
  assert.ok(!result.ok, "expected a failed Result");
  return result.error;
}

test("mcpServers() returns the tenant's registered server names", async () => {
  const loom = clientReturning(
    () =>
      new Response(JSON.stringify({ servers: ["lucidbrain", "workspec"] }), {
        status: 200,
        headers: { "content-type": "application/json" },
      }),
  );
  const result = await loom.mcpServers();
  assert.ok(result.ok, "the request succeeded");
  if (!result.ok) return;
  assert.deepEqual(result.value, ["lucidbrain", "workspec"]);
});

test("mcpServers() surfaces a 401 as a typed http LoomError", async () => {
  const loom = clientReturning(
    () =>
      new Response(
        JSON.stringify({ error: { code: "unauthorized", message: "missing or invalid virtual key" } }),
        { status: 401, statusText: "Unauthorized", headers: { "content-type": "application/json" } },
      ),
  );
  const error = assertErr(await loom.mcpServers());
  assert.equal(error.kind, "http");
  if (error.kind !== "http") return;
  assert.equal(error.status, 401);
  assert.equal(error.code, "unauthorized");
});
