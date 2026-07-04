/**
 * A minimal mock Anthropic Messages API for the LucidBrain integration spike.
 *
 * There is no live Anthropic key in this environment and the Loom release binary
 * has no built-in mock provider, so we register an `anthropic` credential whose
 * `base_url` points here. This server implements just enough of
 * `POST /v1/messages` for Loom's AnthropicProvider to drive it:
 *
 *   - non-streaming: a canned Messages response with a `usage` object;
 *   - streaming (`"stream": true`): the native SSE event sequence
 *     message_start → content_block_start → content_block_delta* →
 *     content_block_stop → message_delta (with final usage) → message_stop.
 *
 * The `usage` deliberately carries `cache_creation_input_tokens` and
 * `cache_read_input_tokens` so the cache write/read split is observable end to
 * end, and the server records every inbound request body so the spike can assert
 * that `mcp_servers` actually reached the provider.
 *
 * Usage: `node anthropic.mjs [port]` (default 8790). Prints the chosen port.
 */

import http from "node:http";

const port = Number(process.argv[2] ?? 8790);

/** Every inbound /v1/messages request body, newest last. */
const received = [];

/**
 * Derives a deterministic usage object from the request, modelling the prompt
 * cache lifecycle: the first turn of a conversation *writes* the cached prefix
 * (`cache_creation_input_tokens`), and every later turn — which resends the
 * cached head — *reads* it (`cache_read_input_tokens`). Caching is only
 * reported when the request actually carries a `cache_control` marker (i.e.
 * auto_cache or an explicit hint reached the provider). This is stateless and
 * run-independent, so repeated spike runs behave identically.
 */
function usageFor(body) {
  const hasCacheControl = JSON.stringify(body).includes("cache_control");
  const priorDepth = Array.isArray(body.messages) ? body.messages.length : 0;
  const firstTurn = priorDepth <= 1;
  return {
    input_tokens: 42,
    output_tokens: 20,
    cache_creation_input_tokens: hasCacheControl && firstTurn ? 128 : 0,
    cache_read_input_tokens: hasCacheControl && !firstTurn ? 128 : 0,
  };
}

/** A short assistant reply that echoes whether MCP servers were attached. */
function replyText(body) {
  const mcp = Array.isArray(body.mcp_servers)
    ? body.mcp_servers.map((s) => s.name).join(",")
    : "";
  return mcp
    ? `Recalled via MCP [${mcp}]: the Titan review shipped on 2026-05-02.`
    : `The Titan review shipped on 2026-05-02.`;
}

function nonStreamingResponse(body) {
  return {
    id: "msg_mock_" + Math.random().toString(36).slice(2, 10),
    type: "message",
    role: "assistant",
    model: body.model ?? "claude-haiku-4-5-20251001",
    content: [{ type: "text", text: replyText(body) }],
    stop_reason: "end_turn",
    stop_sequence: null,
    usage: usageFor(body),
  };
}

function sseFrames(body) {
  const text = replyText(body);
  const usage = usageFor(body);
  const id = "msg_mock_" + Math.random().toString(36).slice(2, 10);
  const model = body.model ?? "claude-haiku-4-5-20251001";
  // Split the reply into a few deltas so the client sees real streaming.
  const words = text.split(" ");
  const chunks = [];
  for (let i = 0; i < words.length; i += 3) {
    chunks.push(words.slice(i, i + 3).join(" ") + (i + 3 < words.length ? " " : ""));
  }

  const events = [];
  events.push([
    "message_start",
    {
      type: "message_start",
      message: {
        id,
        type: "message",
        role: "assistant",
        model,
        content: [],
        stop_reason: null,
        stop_sequence: null,
        usage: {
          input_tokens: usage.input_tokens,
          output_tokens: 0,
          cache_creation_input_tokens: usage.cache_creation_input_tokens,
          cache_read_input_tokens: usage.cache_read_input_tokens,
        },
      },
    },
  ]);
  events.push([
    "content_block_start",
    { type: "content_block_start", index: 0, content_block: { type: "text", text: "" } },
  ]);
  for (const chunk of chunks) {
    events.push([
      "content_block_delta",
      { type: "content_block_delta", index: 0, delta: { type: "text_delta", text: chunk } },
    ]);
  }
  events.push(["content_block_stop", { type: "content_block_stop", index: 0 }]);
  events.push([
    "message_delta",
    {
      type: "message_delta",
      delta: { stop_reason: "end_turn", stop_sequence: null },
      usage: { output_tokens: usage.output_tokens },
    },
  ]);
  events.push(["message_stop", { type: "message_stop" }]);
  return events;
}

const server = http.createServer((req, res) => {
  if (req.method === "GET" && req.url === "/__mock/received") {
    res.writeHead(200, { "content-type": "application/json" });
    res.end(JSON.stringify(received));
    return;
  }
  if (req.method !== "POST" || !req.url.startsWith("/v1/messages")) {
    res.writeHead(404).end();
    return;
  }
  let raw = "";
  req.on("data", (c) => (raw += c));
  req.on("end", () => {
    let body = {};
    try {
      body = JSON.parse(raw || "{}");
    } catch {
      res.writeHead(400, { "content-type": "application/json" });
      res.end(JSON.stringify({ type: "error", error: { message: "bad json" } }));
      return;
    }
    received.push({ at: new Date().toISOString(), body });

    if (body.stream === true) {
      res.writeHead(200, {
        "content-type": "text/event-stream",
        "cache-control": "no-cache",
        connection: "keep-alive",
      });
      for (const [event, data] of sseFrames(body)) {
        res.write(`event: ${event}\n`);
        res.write(`data: ${JSON.stringify(data)}\n\n`);
      }
      res.end();
      return;
    }

    res.writeHead(200, { "content-type": "application/json" });
    res.end(JSON.stringify(nonStreamingResponse(body)));
  });
});

server.listen(port, "127.0.0.1", () => {
  console.log(`mock-anthropic listening on http://127.0.0.1:${port}`);
});
