# @fieldstate/loom-client

A TypeScript client for [Loom](../../README.md), Fieldstate's multi-tenant LLM
gateway. This is a **workspace artefact** — private, not published to npm.

It pairs types generated from Loom's OpenAPI document with a small **fluent
wrapper** so an app talks to the gateway in a few lines:

```ts
import { createLoomClient } from "@fieldstate/loom-client";

const loom = createLoomClient({ baseUrl: "http://127.0.0.1:8080", apiKey });

// A lazily-created, tenant-scoped conversation.
const convo = loom.conversation({
  model: "claude-haiku-4-5-20251001",
  system: "You are LucidBrain's recall agent.",
});
convo.withMcp("lucidbrain").cached();           // mcp_servers=[{name:'lucidbrain'}] + auto_cache

// Non-streaming: -> assistant Message
const message = await convo.send("What did we decide about Titan?");

// Streaming: async iterator of normalised TurnEvents (parsed SSE)
for await (const ev of convo.stream("Summarise that.")) {
  if (ev.kind.type === "content_part_delta" && ev.kind.delta.type === "text") {
    process.stdout.write(ev.kind.delta.text);
  }
}

// Stateless helper (nothing persisted):
const oneShot = await loom.turn({
  model: "claude-haiku-4-5-20251001",
  messages: [{ role: "user", content: [{ type: "text", text: "ping" }] }],
});

// Spend/usage rollups:
const usage = await loom.usage({ group_by: "model" });
```

## Surface

- `createLoomClient({ baseUrl, apiKey, fetch? })` → `LoomClient`
- `LoomClient`: `conversation(init)`, `getConversation(id, page?)`,
  `deleteConversation(id)`, `turn(init)`, `streamTurn(init)`, `usage(params?)`,
  `whoami()`
- `ConversationBuilder` (chainable): `withMcp(name | ref)`, `cached()` /
  `withCache(bool)`, `withServerTool(tool)`, `withTools(...)`, `temperature(n)`,
  `maxTokens(n)`, `withOptions(patch)`, then `send(input)`, `stream(input)`,
  `create()`, `fetch(page?)`, and `buildOptions()` (the pure request shape)

`input` to `send`/`stream` is a `string`, a `ContentPart[]`, or a `Message`.

## Types

- `src/generated.ts` — auto-generated from `openapi.json` via
  `openapi-typescript` (re-exported as `paths` / `components` / `operations`).
  Regenerate with `npm run gen` (needs an `openapi.json` snapshot).
- `src/types.ts` — hand-authored domain types mirroring Loom's Rust serde shapes
  (`ContentPart`, `Message`, `TurnEvent`, `ConversationOptions`, …), because the
  gateway currently renders those rich enums as opaque `Object` in the spec. See
  the spike doc's follow-ups for closing that gap.

## Scripts

| Script | Does |
| --- | --- |
| `npm run typecheck` | `tsc --noEmit` |
| `npm run build` | `tsc` → `dist/` |
| `npm test` | request-shaping unit tests (`node --test`, no network) |
| `npm run gen` | regenerate `src/generated.ts` from `openapi.json` |
| `npm run spike` | run the LucidBrain integration flow (needs a running Loom + mock backend — see `scripts/spike.mts` and `../../docs/spikes/lucidbrain-integration.md`) |

## The `openapi.json` snapshot

`openapi.json` is a committed snapshot of the gateway's `GET /openapi.json`,
captured from a running `loom-server`. Refresh it with:

```bash
curl -s http://127.0.0.1:8080/openapi.json \
  | python3 -m json.tool > openapi.json
npm run gen
```
