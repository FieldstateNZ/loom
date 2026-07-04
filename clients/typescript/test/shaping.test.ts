/**
 * Unit tests for request-shaping — the pure logic of the fluent builder, with
 * no network. A fake transport captures the exact JSON bodies the client would
 * send so we can assert the wire shape (`withMcp` + `cached` → the right
 * `options`, content normalisation, stateless turn body, …). Every network call
 * now returns a `Result`, so the tests assert `.ok` before inspecting.
 */

import { test } from "node:test";
import assert from "node:assert/strict";

import { createLoomClient, toContent } from "../src/index.ts";
import type { ContentPart, Message } from "../src/index.ts";

/** A recorded outbound request. */
interface Recorded {
  method: string;
  url: string;
  body: unknown;
  headers: Record<string, string>;
}

/**
 * Builds a client whose `fetch` records requests and returns canned JSON. The
 * `respond` map keys off `METHOD path` (without query string).
 */
function fakeClient(respond: Record<string, unknown>) {
  const calls: Recorded[] = [];
  const fetchImpl: typeof fetch = async (input, init) => {
    const url = String(input);
    const method = init?.method ?? "GET";
    const headers = (init?.headers ?? {}) as Record<string, string>;
    const body = init?.body ? JSON.parse(init.body as string) : undefined;
    calls.push({ method, url, body, headers });
    const path = new URL(url).pathname;
    const key = `${method} ${path}`;
    const payload = respond[key] ?? {};
    return new Response(JSON.stringify(payload), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };
  const created = createLoomClient({
    baseUrl: "http://loom.test",
    apiKey: "loom_test_key",
    fetch: fetchImpl,
  });
  assert.ok(created.ok, "client config is valid");
  return { loom: created.value, calls };
}

const assistant: Message = {
  role: "assistant",
  content: [{ type: "text", text: "ok" }],
};

/** A schema-valid create-conversation response for the fake transport. */
const conversation = (id: string) => ({
  id,
  tenant_id: "t1",
  binding: { provider: "anthropic", model: "m" },
  messages: [],
});

test("withMcp('lucidbrain') + cached() produce the right options JSON", async () => {
  const { loom, calls } = fakeClient({
    "POST /v1/conversations": conversation("conv-1"),
    "POST /v1/conversations/conv-1/turns": assistant,
  });

  const convo = loom.conversation({ model: "claude-haiku-4-5-20251001" });
  convo.withMcp("lucidbrain").cached();

  // buildOptions is the pure shaping surface.
  assert.deepEqual(convo.buildOptions(), {
    mcp_servers: [{ name: "lucidbrain" }],
    auto_cache: true,
  });

  const sent = await convo.send("recall Titan");
  assert.ok(sent.ok, "the turn succeeded");

  const turn = calls.find((c) => c.url.endsWith("/turns"));
  assert.ok(turn, "a turn request was sent");
  assert.deepEqual(turn.body, {
    content: [{ type: "text", text: "recall Titan" }],
    stream: false,
    options: { mcp_servers: [{ name: "lucidbrain" }], auto_cache: true },
  });
  // Auth is carried as a bearer token.
  assert.equal(turn.headers.authorization, "Bearer loom_test_key");
});

test("conversation is created lazily and reused across turns", async () => {
  const { loom, calls } = fakeClient({
    "POST /v1/conversations": conversation("conv-9"),
    "POST /v1/conversations/conv-9/turns": assistant,
  });
  const convo = loom.conversation({ model: "m", system: "you are Loom" });
  assert.ok((await convo.send("first")).ok);
  assert.ok((await convo.send("second")).ok);

  const creates = calls.filter((c) => c.url.endsWith("/v1/conversations"));
  assert.equal(creates.length, 1, "conversation created exactly once");
  // `metadata` is absent (undefined) so JSON.stringify drops it on the wire.
  assert.deepEqual(creates[0]!.body, {
    provider: "anthropic",
    model: "m",
    system: "you are Loom",
  });
  assert.equal(convo.id, "conv-9");
  const turns = calls.filter((c) => c.url.includes("/turns"));
  assert.equal(turns.length, 2);
});

test("withMcp de-duplicates by name (last wins) and server tools accumulate", () => {
  const { loom } = fakeClient({});
  const convo = loom.conversation({ model: "m" });
  convo
    .withMcp("lucidbrain")
    .withMcp({ name: "lucidbrain", tool_configuration: { allowed: ["recall"] } })
    .withServerTool({ kind: "web_search", max_uses: 3 })
    .temperature(0.4)
    .maxTokens(512);

  assert.deepEqual(convo.buildOptions(), {
    mcp_servers: [
      { name: "lucidbrain", tool_configuration: { allowed: ["recall"] } },
    ],
    server_tools: [{ kind: "web_search", max_uses: 3 }],
    temperature: 0.4,
    max_tokens: 512,
  });
});

test("stateless turn helper shapes the /v1/turns body", async () => {
  const { loom, calls } = fakeClient({ "POST /v1/turns": assistant });
  const messages: Message[] = [
    { role: "user", content: [{ type: "text", text: "hi" }] },
  ];
  const result = await loom.turn({
    model: "m",
    system: "sys",
    messages,
    options: { auto_cache: true },
  });
  assert.ok(result.ok, "the stateless turn succeeded");

  const call = calls.find((c) => c.url.endsWith("/v1/turns"));
  assert.ok(call);
  assert.deepEqual(call.body, {
    provider: "anthropic",
    model: "m",
    system: "sys",
    messages,
    options: { auto_cache: true },
    stream: false,
  });
});

test("toContent normalises strings, parts and messages", () => {
  assert.deepEqual(toContent("hello"), [{ type: "text", text: "hello" }]);
  const parts: ContentPart[] = [{ type: "text", text: "x" }];
  assert.equal(toContent(parts), parts);
  assert.deepEqual(toContent(assistant), assistant.content);
});

test("builder does not mutate the caller's shared options object", () => {
  const { loom } = fakeClient({});
  const shared = { temperature: 0.1 };
  const convo = loom.conversation({ model: "m", options: shared });
  convo.cached().withMcp("lucidbrain");
  assert.deepEqual(shared, { temperature: 0.1 }, "caller options untouched");
});
