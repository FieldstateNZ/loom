// createMockClient — a LoomClient backed by the frozen in-memory seed. The
// design/dev default (used whenever no live base URL is configured); it never
// fails, so its mutations always resolve to `ok(...)`.
import { ok, type Result, type LoomError } from "./result.ts";
import { SNAPSHOT } from "./mock-data.ts";
import { TRANSCRIPT } from "./mock-transcript.ts";
import type { LoomClient } from "./client.ts";
import type { LoomSnapshot } from "./snapshot.ts";
import type { Transcript } from "./transcript.ts";
import type { VirtualKey } from "./models.ts";

/** Deep-clones seed data so callers can never mutate the shared frozen source. */
const clone = <T>(v: T): T => JSON.parse(JSON.stringify(v)) as T;

/** Simulates gateway latency so loading states are exercised in the demo. */
const delay = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

/** Builds a LoomClient that serves the frozen demo seed. */
export function createMockClient(): LoomClient {
  return {
    async bootstrap() {
      await delay(120);
      return clone<LoomSnapshot>(SNAPSHOT);
    },

    async getTranscript(conversationId) {
      await delay(80);
      return conversationId === TRANSCRIPT.id ? clone<Transcript>(TRANSCRIPT) : null;
    },

    async createKey(input): Promise<Result<{ key: VirtualKey; secret: string }, LoomError>> {
      await delay(160);
      const secret = "loom_k1_9f2c4e8a7b3d5f01_" + input.name.replace(/[^a-z0-9]/gi, "_") + "_XA4Q";
      const key: VirtualKey = {
        id: "key_" + Math.floor(SNAPSHOT.stats.requests + Math.abs(hash(input.name))).toString(36),
        name: input.name,
        tenant: input.tenant,
        status: "active",
        scopes: input.scopes,
        budgetSpent: 0,
        cap: input.cap,
        window: input.window,
        mode: input.mode,
        last: "just now",
        spend7: 0,
        rateRpm: 60,
      };
      return ok({ key, secret });
    },

    async revokeKey(id): Promise<Result<VirtualKey, LoomError>> {
      await delay(120);
      const found = SNAPSHOT.keys.find((k) => k.id === id);
      const base: VirtualKey = found
        ? clone<VirtualKey>(found)
        : { id, name: id, tenant: "", status: "active", scopes: [], budgetSpent: 0, cap: null, window: null, mode: "block", last: "", spend7: 0 };
      const revoked: VirtualKey = { ...base, status: "revoked" };
      return ok(revoked);
    },

    async checkProviderConnectivity(providerId) {
      await delay(900);
      const p = SNAPSHOT.providers.find((x) => x.id === providerId);
      if (p && p.status === "connected") {
        return { ok: true, detail: `ok · 214ms round trip · models/list ${p.models} models` };
      }
      return { ok: false, detail: "failed · could not reach provider" };
    },

    async checkMcpConnectivity(serverId) {
      await delay(900);
      const s = SNAPSHOT.mcpServers.find((x) => x.id === serverId);
      if (s && s.status === "connected") {
        return { ok: true, detail: "ok · tools/list returned 12 tools" };
      }
      return { ok: false, detail: "failed · 401 unauthorized — rotate the token" };
    },
  };
}

/** Deterministic id salt (no Math.random, so ids stay stable per key name). */
function hash(s: string): number {
  let h = 5381;
  for (let i = 0; i < s.length; i++) h = (h << 5) + h + s.charCodeAt(i);
  return h | 0;
}
