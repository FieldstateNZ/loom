// The frozen demo transcript for conv_9f2c4e8a — deliberately exercises every
// block type (text, thinking, tool use + result, web search with citations,
// code execution, cache markers, an errored tool call, and the unknown-block
// raw-JSON fallback) so the renderer can be seen end-to-end without a gateway.
import type { Transcript } from "./transcript.ts";

/** The one seeded transcript the mock client can return. */
export const TRANSCRIPT: Transcript = {
  id: "conv_9f2c4e8a",
  key: "lucidbrain-prod",
  model: "claude-sonnet-4-5",
  totals: { cost: 0.0117, inTok: 1844, outTok: 510, cacheRead: 27200, cacheWrite: 3110 },
  turns: [
    { role: "user", time: "14:32:08", blocks: [
      { type: "text", text: "What did we spend on caching last week, and was it worth it?" },
    ] },
    { role: "assistant", time: "14:32:11", model: "claude-sonnet-4-5",
      usage: { cost: 0.0086, inTok: 1204, outTok: 388, cacheRead: 12400, cacheWrite: 3110, ms: 842 },
      blocks: [
        { type: "thinking", duration: "4.2s", text: "The user wants the cache ROI story. Query usage grouped by cache read/write, then compare against what uncached input would have cost at list pricing." },
        { type: "cache", kind: "write", tokens: 3110 },
        { type: "tool_use", name: "query_usage", via: "loom-admin", input: { group_by: "cache", window: "7d" }, result: { cache_read_tok: 8400000, cache_write_tok: 1100000, saved_usd: 18.7 } },
        { type: "web_search", query: "anthropic prompt caching pricing", results: [
          { title: "Prompt caching - Anthropic", url: "docs.anthropic.com/en/docs/prompt-caching", snippet: "Cache reads are billed at a fraction of base input tokens; writes carry a one-time premium.", cited: true },
          { title: "Pricing - Anthropic", url: "anthropic.com/pricing" },
        ] },
        { type: "code_exec", lang: "python", code: "read_cost = 8.4e6 * 0.3 / 1e6\nfull_cost = 8.4e6 * 3.0 / 1e6\nprint(f\"saved ${full_cost - read_cost:.2f}\")", stdout: "saved $22.68", exitCode: 0 },
        { type: "text", text: "Caching saved roughly $22.68 against list input pricing last week — an 87% discount on 8.4M replayed tokens. The write premium cost $4.10, so net ROI is strongly positive.\n\nIf LucidBrain keeps this traffic shape, expect ~$90/month in avoided input spend." },
        { type: "unknown", blockType: "provider_usage_extension", data: { type: "provider_usage_extension", version: 2, payload: { speculative_tokens: 88 } } },
      ] },
    { role: "user", time: "14:33:02", blocks: [
      { type: "text", text: "Chart that by day and post it to the finance channel." },
    ] },
    { role: "assistant", time: "14:33:05", model: "claude-sonnet-4-5",
      usage: { cost: 0.0031, inTok: 640, outTok: 122, cacheRead: 14800, cacheWrite: 0, ms: 610 },
      blocks: [
        { type: "cache", kind: "read", tokens: 14800 },
        { type: "tool_use", name: "post_message", via: "slack-mcp", input: { channel: "#finance", blocks: "…" }, result: "timeout after 30s", isError: true },
        { type: "text", text: "The chart is ready, but posting to Slack failed — slack-mcp timed out. I can retry, or hand you the image to post manually." },
      ] },
  ],
};
