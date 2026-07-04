/**
 * Unit tests for {@link collect} — the stream-to-final-turn accumulator.
 *
 * The first two tests feed a canned `TurnEvent` sequence directly (no network)
 * and assert the assembled `message`/`usage`/`stopReason`/`cost`, mirroring the
 * server's `TurnAccumulator` fold: only `content_part_complete` parts (never
 * `content_part_started`/`content_part_delta`) contribute to the message, and
 * `turn_ended`'s own `usage`/`cost` supersede a bare `usage` event and an
 * absent cost respectively. A third test drives `collect` through a real
 * `LoomClient` and asserts the streamed, collected result — including
 * `cost` — is identical to what the non-streaming `client.turn()` returns
 * (its `TurnResponse`) for the same input — the acceptance criterion for issue
 * #20, extended to cost by #23.
 */

import { test } from "node:test";
import assert from "node:assert/strict";

import { collect, createLoomClient, err, ok } from "../src/index.ts";
import type { LoomError, Message, Result, TurnCost, TurnEvent } from "../src/index.ts";

/** Wraps a plain array of already-built `Result`s as an `AsyncIterable`. */
async function* streamOf(
  events: readonly Result<TurnEvent, LoomError>[],
): AsyncGenerator<Result<TurnEvent, LoomError>, void, unknown> {
  yield* events;
}

/** A minimal, schema-shaped `TurnEvent` — `raw` is never inspected by `collect`. */
function event(kind: TurnEvent["kind"]): Result<TurnEvent, LoomError> {
  return ok({ kind, raw: null });
}

test("collect() folds a text turn: only the complete part counts, terminal usage and cost win", async () => {
  const cost: TurnCost = { amount: "0.0042", currency: "USD" };
  const result = await collect(
    streamOf([
      event({ type: "turn_started" }),
      event({ type: "content_part_started", index: 0, part: { type: "text", text: "" } }),
      event({ type: "content_part_delta", index: 0, delta: { type: "text", text: "Hi" } }),
      event({ type: "content_part_complete", index: 0, part: { type: "text", text: "Hi there" } }),
      event({ type: "usage", input_tokens: 3, output_tokens: 2 }),
      event({
        type: "turn_ended",
        stop_reason: "end_turn",
        usage: { input_tokens: 3, output_tokens: 4 },
        cost,
      }),
    ]),
  );

  assert.ok(result.ok, "collect succeeded");
  const expected: Message = {
    role: "assistant",
    content: [{ type: "text", text: "Hi there" }],
    usage: { input_tokens: 3, output_tokens: 4 },
  };
  assert.deepEqual(result.value.message, expected);
  assert.deepEqual(result.value.usage, { input_tokens: 3, output_tokens: 4 });
  assert.equal(result.value.stopReason, "end_turn");
  assert.deepEqual(result.value.cost, cost);
});

test("collect() folds a tool-use turn from out-of-order indices, with no cost reported", async () => {
  const result = await collect(
    streamOf([
      event({
        type: "content_part_complete",
        index: 1,
        part: { type: "text", text: "let me check" },
      }),
      event({
        type: "content_part_complete",
        index: 0,
        part: { type: "tool_use", id: "call_1", name: "get_weather", input: { city: "NYC" } },
      }),
      event({
        type: "turn_ended",
        stop_reason: "tool_use",
        usage: { input_tokens: 12, output_tokens: 1 },
      }),
    ]),
  );

  assert.ok(result.ok, "collect succeeded");
  assert.deepEqual(result.value.message.content, [
    { type: "tool_use", id: "call_1", name: "get_weather", input: { city: "NYC" } },
    { type: "text", text: "let me check" },
  ]);
  assert.equal(result.value.stopReason, "tool_use");
  assert.deepEqual(result.value.usage, { input_tokens: 12, output_tokens: 1 });
  // No `cost` on the turn_ended event (e.g. no price configured) folds to `null`.
  assert.equal(result.value.cost, null);
});

test("collect() short-circuits on a mid-stream LoomStreamError", async () => {
  const streamError: LoomError = { kind: "stream", code: "provider_error", message: "boom" };
  const result = await collect(
    streamOf([
      event({ type: "content_part_complete", index: 0, part: { type: "text", text: "partial" } }),
      err(streamError),
      event({ type: "turn_ended", stop_reason: "end_turn" }),
    ]),
  );

  assert.equal(result.ok, false, "collect propagated the stream failure");
  assert.deepEqual(!result.ok && result.error, streamError);
});

test("collect() reports a decode error when the stream ends without turn_ended", async () => {
  const result = await collect(
    streamOf([
      event({ type: "content_part_complete", index: 0, part: { type: "text", text: "hi" } }),
    ]),
  );

  assert.equal(result.ok, false, "an incomplete stream is a failure");
  assert.equal(!result.ok && result.error.kind, "decode");
});

test("collect(client.streamTurn(init)) equals client.turn(init) for the same input, cost included", async () => {
  const finalMessage: Message = {
    role: "assistant",
    content: [{ type: "text", text: "Hello, world" }],
    usage: { input_tokens: 10, output_tokens: 6 },
  };
  // The authoritative priced cost the gateway would compute for this turn —
  // identical on both the non-streaming envelope and the streamed
  // `turn_ended` event, since both derive from the same server-side price.
  const cost: TurnCost = { amount: "0.0123", currency: "USD" };
  const frames: readonly TurnEvent["kind"][] = [
    { type: "turn_started" },
    { type: "content_part_started", index: 0, part: { type: "text", text: "" } },
    { type: "content_part_delta", index: 0, delta: { type: "text", text: "Hello" } },
    { type: "content_part_complete", index: 0, part: { type: "text", text: "Hello, world" } },
    { type: "usage", input_tokens: 10, output_tokens: 5 },
    {
      type: "turn_ended",
      stop_reason: "end_turn",
      usage: { input_tokens: 10, output_tokens: 6 },
      cost,
    },
  ];

  const fetchImpl: typeof fetch = async (_input, init) => {
    const body = init?.body ? (JSON.parse(init.body as string) as { stream?: boolean }) : undefined;
    if (body?.stream) {
      const sse = frames.map((kind) => `data: ${JSON.stringify({ kind, raw: null })}\n\n`).join("");
      const stream = new ReadableStream<Uint8Array>({
        start(controller) {
          controller.enqueue(new TextEncoder().encode(sse));
          controller.close();
        },
      });
      return new Response(stream, { status: 200, headers: { "content-type": "text/event-stream" } });
    }
    return new Response(JSON.stringify({ message: finalMessage, cost }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };

  const created = createLoomClient({ baseUrl: "http://loom.test", apiKey: "k", fetch: fetchImpl });
  assert.ok(created.ok, "client config is valid");
  const loom = created.value;

  const init = { model: "m", messages: [{ role: "user" as const, content: [{ type: "text" as const, text: "hi" }] }] };
  const turned = await loom.turn(init);
  assert.ok(turned.ok, "the non-streaming turn succeeded");

  const collected = await collect(loom.streamTurn(init));
  assert.ok(collected.ok, "collect succeeded");
  assert.deepEqual(collected.value.message, turned.value.message);
  // The acceptance criterion extended to cost (#23): streamed == non-streamed.
  assert.deepEqual(collected.value.cost, turned.value.cost);
  assert.deepEqual(turned.value.cost, cost);
});
