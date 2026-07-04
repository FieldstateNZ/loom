/**
 * The LucidBrain integration spike.
 *
 * Drives a representative memory-recall flow through Loom using the fluent
 * client, against a local Postgres + loom-server + a mock Anthropic backend
 * (see mock/anthropic.mjs). It:
 *
 *   1. provisions a tenant, a virtual key and an `anthropic` credential whose
 *      base_url points at the mock (via `/admin`, root token);
 *   2. runs a multi-turn conversation referencing an MCP server `lucidbrain`
 *      (withMcp), a streaming turn (consuming the async iterator), and a cached
 *      turn (auto_cache);
 *   3. asserts the MCP ref reached the provider, streaming yielded TurnEvents,
 *      and `/v1/usage` shows the cache read/write split;
 *   4. measures Loom's latency overhead vs hitting the mock directly (N runs).
 *
 * Prints a JSON report to stdout. Exits non-zero on any assertion failure.
 */

import { createLoomClient } from "../src/index.ts";
import type { TurnEvent } from "../src/index.ts";

const LOOM_URL = process.env.LOOM_URL ?? "http://127.0.0.1:8080";
const MOCK_URL = process.env.MOCK_URL ?? "http://127.0.0.1:8790";
const ROOT_TOKEN = process.env.ROOT_TOKEN ?? "spike-root";
const MODEL = process.env.MODEL ?? "claude-haiku-4-5-20251001";
const N = Number(process.env.N ?? 60);

function assert(cond: unknown, msg: string): asserts cond {
  if (!cond) throw new Error(`ASSERT FAILED: ${msg}`);
}

async function admin<T>(method: string, path: string, body?: unknown): Promise<T> {
  const res = await fetch(`${LOOM_URL}${path}`, {
    method,
    headers: {
      authorization: `Bearer ${ROOT_TOKEN}`,
      "content-type": "application/json",
    },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
  });
  if (!res.ok) throw new Error(`admin ${method} ${path} -> ${res.status}: ${await res.text()}`);
  const text = await res.text();
  return (text ? JSON.parse(text) : undefined) as T;
}

interface Stats {
  mean: number;
  p50: number;
  p95: number;
}
function stats(samples: number[]): Stats {
  const s = [...samples].sort((a, b) => a - b);
  const q = (p: number) => s[Math.min(s.length - 1, Math.floor(p * (s.length - 1)))]!;
  const mean = s.reduce((a, b) => a + b, 0) / s.length;
  return { mean: round(mean), p50: round(q(0.5)), p95: round(q(0.95)) };
}
const round = (n: number) => Math.round(n * 100) / 100;

async function main() {
  const report: Record<string, unknown> = {};

  // 1. Provision -----------------------------------------------------------
  const slug = `lucidbrain-${Date.now()}`;
  const tenant = await admin<{ id: string }>("POST", "/admin/tenants", {
    slug,
    name: "LucidBrain Spike",
  });
  const keyResp = await admin<{ key: string; key_prefix: string }>(
    "POST",
    `/admin/tenants/${tenant.id}/keys`,
    { name: "spike-key" },
  );
  await admin("PUT", `/admin/tenants/${tenant.id}/credentials/anthropic`, {
    api_key: "test",
    base_url: MOCK_URL,
  });
  // A *named* MCP reference (`withMcp('lucidbrain')`) resolves against the
  // tenant's registered servers, so the server must be registered out-of-band
  // first. The gateway injects this URL + token into the provider request
  // server-side; the mock never actually dials it.
  await admin("PUT", `/admin/tenants/${tenant.id}/mcp-servers/lucidbrain`, {
    url: `${MOCK_URL}/mcp/lucidbrain`,
    authorization_token: "lucidbrain-mcp-token",
  });
  report.tenant_id = tenant.id;
  report.key_prefix = keyResp.key_prefix;

  const created = createLoomClient({ baseUrl: LOOM_URL, apiKey: keyResp.key });
  assert(created.ok, "client config is valid");
  const loom = created.value;

  const who = await loom.whoami();
  assert(who.ok, "whoami succeeded");
  assert(who.value.tenant_id === tenant.id, "whoami resolves the minted key to the tenant");

  // 2. Representative flow --------------------------------------------------
  // Multi-turn, memory-recall-shaped conversation referencing the lucidbrain
  // MCP server, with prompt caching enabled.
  const convo = loom
    .conversation({
      model: MODEL,
      system: "You are LucidBrain's recall agent. Cite retrieved memories.",
    })
    .withMcp("lucidbrain")
    .cached();

  const turn1 = await convo.send("What did we decide about the Titan project?");
  assert(turn1.ok, "turn 1 succeeded");
  assert(turn1.value.role === "assistant", "turn 1 returns an assistant message");
  const turn1Text = turn1.value.content.find((p) => p.type === "text");
  report.turn1_text = turn1Text && "text" in turn1Text ? turn1Text.text : null;
  report.turn1_usage = turn1.value.usage ?? null;

  const turn2 = await convo.send("And who owned the follow-up?");
  assert(turn2.ok, "turn 2 succeeded");
  assert(turn2.value.role === "assistant", "turn 2 returns an assistant message");
  report.turn2_usage = turn2.value.usage ?? null;

  // Streaming turn: consume the async iterator of TurnEvents.
  const events: TurnEvent[] = [];
  let streamedText = "";
  for await (const ev of convo.stream("Summarise that for the standup.")) {
    assert(ev.ok, "stream frame decoded without error");
    const event = ev.value;
    events.push(event);
    if (event.kind.type === "content_part_delta" && event.kind.delta.type === "text") {
      streamedText += event.kind.delta.text;
    }
  }
  assert(events.length > 0, "streaming yielded TurnEvents");
  const kinds = new Set(events.map((e) => e.kind.type));
  assert(kinds.has("turn_started"), "stream includes turn_started");
  assert(kinds.has("content_part_delta"), "stream includes content_part_delta");
  assert(kinds.has("turn_ended"), "stream includes turn_ended");
  const ended = events.find((e) => e.kind.type === "turn_ended");
  report.stream_event_count = events.length;
  report.stream_event_kinds = [...kinds];
  report.stream_text = streamedText;
  report.stream_end_usage =
    ended && ended.kind.type === "turn_ended" ? ended.kind.usage ?? null : null;

  // Fetch the persisted conversation history (user + assistant turns).
  const history = await convo.fetch();
  assert(history.ok, "history fetch succeeded");
  report.persisted_message_count = history.value.messages.length;

  // 3. Assert the MCP ref reached the provider (via the mock's recorder). ----
  const mockReceived = (await (await fetch(`${MOCK_URL}/__mock/received`)).json()) as Array<{
    body: { mcp_servers?: Array<{ name: string; url?: string }>; system?: unknown };
  }>;
  assert(mockReceived.length > 0, "mock received provider requests");
  const withMcp = mockReceived.filter(
    (r) => Array.isArray(r.body.mcp_servers) && r.body.mcp_servers.some((s) => s.name === "lucidbrain"),
  );
  assert(withMcp.length >= 3, "the lucidbrain MCP ref reached the provider on every turn");
  // Named MCP refs are resolved server-side: the URL is injected by the gateway.
  report.mcp_ref_seen_by_provider = withMcp[0]!.body.mcp_servers;

  // 3b. Usage rollup shows the priced cache read/write split. ---------------
  const usage = await loom.usage({ group_by: "model" });
  assert(usage.ok, "usage rollup succeeded");
  report.usage_rollup = usage.value;
  const modelRow = usage.value.rows.find((r) => r.group === MODEL);
  assert(modelRow, "usage rollup has a row for the model");
  assert(modelRow.event_count >= 3, "usage rollup counted the turns");
  assert(
    modelRow.cache_write_tokens > 0,
    "usage rollup shows cache-write (creation) tokens",
  );
  assert(
    modelRow.cache_read_tokens > 0,
    "usage rollup shows cache-read tokens (cache was hit on a later turn)",
  );

  // 4. Latency overhead: Loom vs direct-to-mock, N runs each. ---------------
  // Warm both paths first (JIT, connection setup) so the first-call cost does
  // not skew the sample.
  const warm = loom.conversation({ model: MODEL }).withMcp("lucidbrain");
  await warm.send("warmup");
  await fetch(`${MOCK_URL}/v1/messages`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ model: MODEL, max_tokens: 64, messages: [] }),
  });

  const loomSamples: number[] = [];
  for (let i = 0; i < N; i++) {
    const c = loom.conversation({ model: MODEL }).withMcp("lucidbrain");
    // Pre-create so we time only the turn, matching the single-request direct call.
    await c.create();
    const t0 = performance.now();
    await c.send("ping");
    loomSamples.push(performance.now() - t0);
  }

  const directBody = JSON.stringify({
    model: MODEL,
    max_tokens: 64,
    messages: [{ role: "user", content: "ping" }],
  });
  const directSamples: number[] = [];
  for (let i = 0; i < N; i++) {
    const t0 = performance.now();
    const r = await fetch(`${MOCK_URL}/v1/messages`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: directBody,
    });
    await r.text();
    directSamples.push(performance.now() - t0);
  }

  const loomStats = stats(loomSamples);
  const directStats = stats(directSamples);
  report.latency = {
    n: N,
    unit: "ms",
    loom: loomStats,
    direct_to_mock: directStats,
    overhead: {
      mean: round(loomStats.mean - directStats.mean),
      p50: round(loomStats.p50 - directStats.p50),
      p95: round(loomStats.p95 - directStats.p95),
    },
  };

  report.ok = true;
  console.log(JSON.stringify(report, null, 2));
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
