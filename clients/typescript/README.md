# @fieldstate/loom-client

A TypeScript client for [Loom](../../README.md), Fieldstate's multi-tenant LLM
gateway. This is a **workspace artefact** — private, not published to npm.

It pairs types generated from Loom's OpenAPI document with a small **fluent
wrapper** so an app talks to the gateway in a few lines. Every fallible call
returns a **`Result`** rather than throwing, so callers branch on `ok` and read a
structured `LoomError` on failure.

```ts
import { createLoomClient } from "@fieldstate/loom-client";

// createLoomClient validates the config (Zod) and returns a Result.
const created = createLoomClient({ baseUrl: "http://127.0.0.1:8080", apiKey });
if (!created.ok) throw new Error(created.error.message);
const loom = created.value;

// A lazily-created, tenant-scoped conversation.
const convo = loom.conversation({
  model: "claude-haiku-4-5-20251001",
  system: "You are LucidBrain's recall agent.",
});
convo.withMcp("lucidbrain").cached();           // mcp_servers=[{name:'lucidbrain'}] + auto_cache

// Non-streaming: -> Result<Message, LoomError>
const reply = await convo.send("What did we decide about Titan?");
if (!reply.ok) {
  console.error(reply.error.code, reply.error.message);
} else {
  // reply.value is the assistant Message
}

// Streaming: async iterator of Result<TurnEvent, LoomError> (parsed SSE)
for await (const ev of convo.stream("Summarise that.")) {
  if (!ev.ok) break; // a terminal error frame or a malformed frame
  const { kind } = ev.value;
  if (kind.type === "content_part_delta" && kind.delta.type === "text") {
    process.stdout.write(kind.delta.text);
  }
}

// Stateless helper (nothing persisted): -> Result<Message, LoomError>
const oneShot = await loom.turn({
  model: "claude-haiku-4-5-20251001",
  messages: [{ role: "user", content: [{ type: "text", text: "ping" }] }],
});

// Spend/usage rollups: -> Result<UsageRollupResponse, LoomError>
const usage = await loom.usage({ group_by: "model" });
```

## Errors: the `Result` pattern

Expected failures — a rejected key (401), an unknown conversation (404), an
exhausted budget (402), a provider hiccup (502), a malformed response — are
returned, never thrown. Throwing is reserved for genuine programmer/environment
error (e.g. a runtime with no `fetch`).

- `type Result<T, E> = { ok: true; value: T } | { ok: false; error: E }`, with
  `ok()` / `err()` constructors.
- `LoomError` is a discriminated union on `kind`:
  - `"http"` — the gateway's `{ code, message, provider_error?, details? }`
    envelope plus the HTTP `status`;
  - `"network"` — `fetch` never produced a response;
  - `"decode"` — a 2xx body failed JSON parsing or schema validation;
  - `"stream"` — a terminal `error` frame arrived mid-stream;
  - `"config"` — the client config failed validation.

## Surface

- `createLoomClient({ baseUrl, apiKey, fetch?, logger? })` →
  `Result<LoomClient, LoomError>`
- `LoomClient`: `conversation(init)`, `getConversation(id, page?)`,
  `deleteConversation(id)`, `turn(init)`, `streamTurn(init)`, `usage(params?)`,
  `whoami()` — each network method returns a `Result` (streams yield `Result`s)
- `ConversationBuilder` (chainable): `withMcp(name | ref)`, `cached()` /
  `withCache(bool)`, `withServerTool(tool)`, `withTools(...)`, `temperature(n)`,
  `maxTokens(n)`, `withOptions(patch)`, then `send(input)`, `stream(input)`,
  `create()`, `fetch(page?)`, and `buildOptions()` (the pure request shape)

`input` to `send`/`stream` is a `string`, a `ContentPart[]`, or a `Message`.

## Types & validation

- `src/generated.ts` — auto-generated from `openapi.json` via
  `openapi-typescript` (re-exported as `paths` / `components` / `operations`).
  Regenerate with `npm run gen` (needs an `openapi.json` snapshot). **Never
  hand-edit** this file.
- `src/models/` — the hand-authored domain models, one per concern
  (`ContentPart`, `Message`, `TurnEvent`, `ConversationOptions`, …). Response
  shapes are **Zod schemas** with their TS types derived via `z.infer`, so the
  schema is the single source of truth and untrusted API/SSE JSON is validated
  at the boundary. Request-only shapes are plain `readonly` types.

The gateway currently renders its rich `#[non_exhaustive]` enums as opaque
`Object` in the OpenAPI spec, which is why these models are authored by hand.

## Scripts

| Script | Does |
| --- | --- |
| `npm run typecheck` | `tsc --noEmit` over `src`, `test`, and `scripts` |
| `npm run build` | `tsc -p tsconfig.build.json` → `dist/` (src only) |
| `npm run lint` | `eslint .` (flat config in `eslint.config.js`) |
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
