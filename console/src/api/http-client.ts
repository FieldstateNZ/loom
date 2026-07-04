// createHttpClient — a LoomClient backed by the live Loom gateway's REST API.
//
// Satisfies the same LoomClient interface the mock does, against a running
// gateway. Reads/mutations go through a Result-returning transport (see
// http-request.ts); expected failures surface as `err(...)` for the UI to read,
// not thrown exceptions. Snapshot assembly and transcript mapping live in
// sibling modules to keep each file small and single-concern.
import { asRecord, str } from "./json.ts";
import { ok, err, loomError, type Result, type LoomError } from "./result.ts";
import { createRequest } from "./http-request.ts";
import { buildBootstrap } from "./bootstrap.ts";
import { mapMessage, type PendingBlock } from "./transcript-map.ts";
import { transcriptTotals } from "./transcript-totals.ts";
import type { LoomClient } from "./client.ts";
import type { Transcript } from "./transcript.ts";
import type { VirtualKey } from "./models.ts";
import type { CreateKeyInput, ConnectivityResult } from "./snapshot.ts";

/** Configuration for {@link createHttpClient}. */
export interface HttpClientOptions {
  /** The gateway's base URL, e.g. `https://gateway.example.com`. */
  readonly baseUrl: string;
  /** The root admin token, for the `/admin` surface (key/tenant provisioning). */
  readonly adminToken?: string | undefined;
  /** A tenant virtual key (`loom_…`), for the tenant-scoped `/v1` surface. */
  readonly apiKey?: string | undefined;
}

/**
 * The console capabilities the live gateway cannot satisfy yet — surfaced so the
 * gap is honest and discoverable (mirrored in console/README) rather than
 * silently faked. Kept as data, not logged, so shipped code stays `console`-free.
 */
export const HTTP_CLIENT_GAPS: readonly string[] = [
  "bootstrap().keys is empty — no list-all-keys endpoint on the gateway",
  "bootstrap().providers / credOverrides are empty — no provider list/read endpoint",
  "bootstrap().tenants omits key-count, budget cap/window and block counts — no read endpoints",
  "bootstrap().mcpServers status is registration-only — no live connectivity probe",
  "bootstrap().spendByHour / priorByHour / usageDaily are empty — no time-series endpoint",
  "bootstrap().events is empty — no gateway events endpoint",
  "revokeKey() returns a minimal record — the gateway replies 204 with no body and has no read-back",
  "createKey() cannot set scopes — the gateway has no scope-assignment endpoint",
  "checkProviderConnectivity / checkMcpConnectivity are unsupported — no probe endpoints",
];

/** Builds a {@link LoomClient} that talks to the live gateway at `opts.baseUrl`. */
export function createHttpClient(opts: HttpClientOptions): LoomClient {
  const request = createRequest(opts.baseUrl);
  const { adminToken, apiKey } = opts;

  return {
    bootstrap: () => buildBootstrap(request, { adminToken, apiKey }),

    async getTranscript(conversationId: string): Promise<Transcript | null> {
      // Without a virtual key the tenant-scoped endpoint is unreachable.
      if (!apiKey) return null;
      const res = await request(`/v1/conversations/${conversationId}`, { token: apiKey, allow404: true });
      const conv = res.ok ? asRecord(res.value) : null;
      if (!conv) return null;

      const model = str(asRecord(conv.binding)?.model) ?? "";
      const key = str(asRecord(conv.metadata)?.key) ?? "";
      const messages = Array.isArray(conv.messages) ? (conv.messages as unknown[]) : [];
      const pending = new Map<string, PendingBlock>();
      const turns = messages.map((raw) => mapMessage(asRecord(raw) ?? {}, model, pending));
      const totals = await transcriptTotals(request, apiKey, conversationId, turns);
      return { id: str(conv.id) ?? conversationId, key, model, totals, turns };
    },

    async createKey(input: CreateKeyInput): Promise<Result<{ key: VirtualKey; secret: string }, LoomError>> {
      if (!adminToken) return err(loomError("createKey requires an admin token (VITE_LOOM_ADMIN_TOKEN).", "config"));
      const res = await request(`/admin/tenants/${input.tenant}/keys`, {
        method: "POST",
        token: adminToken,
        body: { name: input.name, env: "live" },
      });
      if (!res.ok) return err(res.error);
      const created = asRecord(res.value);
      if (!created) return err(loomError("createKey: empty response from gateway", "parse"));

      const id = str(created.id) ?? "";
      const secret = str(created.key) ?? "";
      // Apply the requested budget best-effort (a separate admin call); the key
      // exists regardless of whether the budget PUT succeeds.
      if (id && input.cap !== null && input.window !== null) {
        await request(`/admin/keys/${id}/budget`, {
          method: "PUT",
          token: adminToken,
          body: { limit_amount: input.cap, window: input.window, action: input.mode },
        });
      }
      const key: VirtualKey = {
        id,
        name: str(created.name) ?? input.name,
        tenant: input.tenant,
        status: "active",
        // Echoes the requested scopes: the gateway has no scope-assignment
        // endpoint, so these reflect intent, not persisted state.
        scopes: input.scopes,
        budgetSpent: 0,
        cap: input.cap,
        window: input.window,
        mode: input.mode,
        last: "just now",
        spend7: 0,
      };
      return ok({ key, secret });
    },

    async revokeKey(id: string): Promise<Result<VirtualKey, LoomError>> {
      if (!adminToken) return err(loomError("revokeKey requires an admin token (VITE_LOOM_ADMIN_TOKEN).", "config"));
      const res = await request(`/admin/keys/${id}`, { method: "DELETE", token: adminToken });
      if (!res.ok) return err(res.error);
      // The gateway replies 204 with no body and exposes no key read-back, so we
      // can only report the revoked status against the id.
      const key: VirtualKey = {
        id,
        name: id,
        tenant: "",
        status: "revoked",
        scopes: [],
        budgetSpent: 0,
        cap: null,
        window: null,
        mode: "block",
        last: "just now",
        spend7: 0,
      };
      return ok(key);
    },

    async checkProviderConnectivity(_providerId: string): Promise<ConnectivityResult> {
      return { ok: false, detail: "unsupported — the gateway exposes no provider connectivity probe endpoint yet" };
    },

    async checkMcpConnectivity(_serverId: string): Promise<ConnectivityResult> {
      return { ok: false, detail: "unsupported — the gateway exposes no MCP connectivity probe endpoint yet" };
    },
  };
}
