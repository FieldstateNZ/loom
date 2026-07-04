// The frozen demo snapshot — the design bundle's dataset (ui_kits/loom-console/
// data.js), captured 2026-07-04 14:53 NZST. It lets the console run and look
// right with no gateway attached; the live client returns the same shape.
import type { LoomSnapshot } from "./snapshot.ts";

/** The seed snapshot the mock client hands out (deep-cloned per call). */
export const SNAPSHOT: LoomSnapshot = {
  now: "14:53:12 NZST · Jul 4",
  keys: [
    { id: "key_01", name: "lucidbrain-prod", tenant: "lucidbrain", status: "active", scopes: ["messages", "streaming", "mcp"], budgetSpent: 38.2, cap: 50, window: "daily", mode: "block", last: "18s ago", spend7: 52.3, rateRpm: 120 },
    { id: "key_02", name: "workspec-prod", tenant: "workspec", status: "active", scopes: ["messages", "streaming"], budgetSpent: 12.9, cap: 75, window: "weekly", mode: "block", last: "2m ago", spend7: 18.44, rateRpm: 60 },
    { id: "key_03", name: "atrium-staging", tenant: "atrium", status: "blocked", scopes: ["messages"], budgetSpent: 20, cap: 20, window: "daily", mode: "block", last: "1h ago", spend7: 6.1, rateRpm: 30 },
    { id: "key_04", name: "lucidbrain-dev", tenant: "lucidbrain", status: "active", scopes: ["messages", "mcp"], budgetSpent: 0.31, cap: null, window: null, mode: "warn", last: "3d ago", spend7: 0.84, rateRpm: 60 },
    { id: "key_05", name: "workspec-dev", tenant: "workspec", status: "revoked", scopes: ["messages"], budgetSpent: 0, cap: 10, window: "daily", mode: "warn", last: "21d ago", spend7: 0, rateRpm: 60 },
  ],
  tenants: [
    { id: "lucidbrain", name: "LucidBrain", status: "active", keys: 2, mcp: 2, spend30: 214.2, cap: 600, window: "monthly", share: 0.62, requests30: 148200, blocks30: 0 },
    { id: "workspec", name: "WorkSpec", status: "active", keys: 2, mcp: 1, spend30: 88.1, cap: 400, window: "monthly", share: 0.27, requests30: 61400, blocks30: 0 },
    { id: "atrium", name: "Atrium", status: "active", keys: 1, mcp: 0, spend30: 24.66, cap: 100, window: "monthly", share: 0.11, requests30: 20900, blocks30: 4 },
  ],
  credOverrides: [
    { tenant: "lucidbrain", provider: "anthropic", set: true, meta: "Rotated 8 days ago", baseUrl: null },
    { tenant: "workspec", provider: "anthropic", set: false, meta: null, baseUrl: null },
    { tenant: "atrium", provider: "anthropic", set: false, meta: null, baseUrl: null },
  ],
  providers: [
    { id: "anthropic", name: "Anthropic", api: "native", status: "connected", keyMeta: "Rotated 8 days ago", baseUrl: null, defaultBaseUrl: "https://api.anthropic.com", models: 34, lastCheck: "ok · 214ms · 4m ago" },
  ],
  stats: {
    spendToday: 14.03, spendPrior: 11.41, spendDelta: 23,
    tokensIn: 8400000, tokensInDelta: 18, tokensOut: 1200000, tokensOutDelta: -4,
    requests: 8043, requestsDelta: 6, streams: 3,
    cacheReadToday: 6200000, cacheWriteToday: 840000, cacheSavedToday: 16.4, cacheHitRate: 0.64,
  },
  spendByHour: [0.22, 0.18, 0.14, 0.11, 0.09, 0.12, 0.24, 0.41, 0.66, 0.81, 0.92, 0.98, 1.04, 0.99, 1.12, 0.71, 0.55, 0.62, 0.58, 0.49, 0.44, 0.38, 0.31, 0.29],
  priorByHour: [0.19, 0.16, 0.13, 0.1, 0.1, 0.11, 0.2, 0.34, 0.52, 0.61, 0.7, 0.74, 0.81, 0.77, 0.8, 0.63, 0.5, 0.52, 0.47, 0.41, 0.36, 0.33, 0.28, 0.24],
  usageDaily: {
    labels: ["Jun 28", "Jun 29", "Jun 30", "Jul 1", "Jul 2", "Jul 3", "Jul 4"],
    cost: [9.4, 10.1, 8.8, 11.6, 12.4, 11.41, 14.03],
    input: [3.1e6, 3.4e6, 2.9e6, 3.8e6, 4.1e6, 3.9e6, 4.4e6],
    output: [0.9e6, 1.0e6, 0.8e6, 1.1e6, 1.2e6, 1.1e6, 1.2e6],
    cacheRead: [3.8e6, 4.2e6, 3.9e6, 4.9e6, 5.4e6, 5.1e6, 6.2e6],
    cacheWrite: [0.7e6, 0.6e6, 0.7e6, 0.8e6, 0.9e6, 0.8e6, 0.84e6],
  },
  topModels: [
    { label: "claude-sonnet-4-5", value: 8.9, display: "$8.90", color: "var(--series-1)" },
    { label: "claude-opus-4-5", value: 3.1, display: "$3.10", color: "var(--series-4)" },
    { label: "claude-haiku-4-5", value: 2.03, display: "$2.03", color: "var(--series-2)" },
  ],
  topKeys: [
    { label: "lucidbrain-prod", value: 8.12, display: "$8.12" },
    { label: "workspec-prod", value: 3.44, display: "$3.44" },
    { label: "atrium-staging", value: 1.87, display: "$1.87" },
    { label: "lucidbrain-dev", value: 0.6, display: "$0.60" },
  ],
  events: [
    { time: "14:32", kind: "block", tone: "danger", key: "atrium-staging", detail: "Hit $20 daily cap — requests refused until 00:00 UTC" },
    { time: "14:07", kind: "error", tone: "warn", key: "lucidbrain-prod", detail: "529 overloaded from provider · retried ×2 · succeeded" },
    { time: "13:44", kind: "warn", tone: "warn", key: "atrium-staging", detail: "Crossed 75% of daily cap" },
    { time: "11:20", kind: "error", tone: "warn", key: "workspec-prod", detail: "MCP slack-mcp unreachable · tool calls failing" },
  ],
  usageByKey: [
    { key: "lucidbrain-prod", requests: 5204, input: 5.2e6, output: 0.71e6, cacheRead: 4.4e6, cacheWrite: 0.5e6, cost: 8.12 },
    { key: "workspec-prod", requests: 1911, input: 2.1e6, output: 0.33e6, cacheRead: 1.2e6, cacheWrite: 0.2e6, cost: 3.44 },
    { key: "atrium-staging", requests: 704, input: 0.8e6, output: 0.11e6, cacheRead: 0.4e6, cacheWrite: 0.09e6, cost: 1.87 },
    { key: "lucidbrain-dev", requests: 224, input: 0.3e6, output: 0.05e6, cacheRead: 0.2e6, cacheWrite: 0.05e6, cost: 0.6 },
  ],
  mcpServers: [
    { id: "mcp_01", tenant: "lucidbrain", name: "github-mcp", url: "https://mcp.internal.fieldstate.nz/github", status: "connected", last: "4m ago", tokenMeta: "Rotated 12 days ago" },
    { id: "mcp_02", tenant: "lucidbrain", name: "slack-mcp", url: "https://mcp.internal.fieldstate.nz/slack", status: "error", last: "1h ago", tokenMeta: "Set 61 days ago" },
    { id: "mcp_03", tenant: "workspec", name: "quire-mcp", url: "https://mcp.internal.fieldstate.nz/quire", status: "connected", last: "22m ago", tokenMeta: "Rotated 3 days ago" },
  ],
  conversations: [
    { id: "conv_9f2c4e8a", key: "lucidbrain-prod", model: "claude-sonnet-4-5", turns: 4, last: "14:33", cost: 0.0117, tokens: 32400, preview: "What did we spend on caching last week, and was it worth it?" },
    { id: "conv_1b3d70aa", key: "workspec-prod", model: "claude-sonnet-4-5", turns: 12, last: "14:21", cost: 0.0844, tokens: 148900, preview: "Draft the compliance summary for the Q2 case-management review…" },
    { id: "conv_77aa02c1", key: "atrium-staging", model: "claude-haiku-4-5", turns: 2, last: "13:58", cost: 0.0031, tokens: 8100, preview: "Classify these support tickets by product area and urgency…", blocked: true },
    { id: "conv_c04e11f9", key: "lucidbrain-prod", model: "claude-opus-4-5", turns: 7, last: "12:40", cost: 0.2210, tokens: 301200, preview: "Plan the migration of the retrieval pipeline to the new embeddings…" },
  ],
};
