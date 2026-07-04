// LoomClient — the console's view of Loom's admin + usage REST API.
//
// The console codes exclusively against this interface. `createMockClient()`
// (mock.ts) satisfies it from a frozen in-memory seed for design/dev; a real
// deployment drops in an HTTP implementation (e.g. `createHttpClient(baseUrl)`)
// that fetches Loom's OpenAPI endpoints. Nothing else in the app changes.

import type {
  LoomSnapshot,
  Transcript,
  VirtualKey,
  CreateKeyInput,
  ConnectivityResult,
} from "./types.ts";

export interface LoomClient {
  /** GET the aggregate dashboard/collection snapshot the console boots from.
   *  Maps onto the gateway's several admin/usage endpoints. */
  bootstrap(): Promise<LoomSnapshot>;

  /** GET /conversations/:id — the turn-by-turn transcript, or null if absent. */
  getTranscript(conversationId: string): Promise<Transcript | null>;

  /** POST /keys — issue a virtual key. The plaintext secret is returned exactly
   *  once (the shown-once moment); the gateway never exposes it again. */
  createKey(input: CreateKeyInput): Promise<{ key: VirtualKey; secret: string }>;

  /** DELETE /keys/:id — revoke a key. Returns the updated record. */
  revokeKey(id: string): Promise<VirtualKey>;

  /** POST /providers/:id/check — provider credential connectivity probe. */
  checkProviderConnectivity(providerId: string): Promise<ConnectivityResult>;

  /** POST /mcp/:id/check — MCP server tools/list reachability probe. */
  checkMcpConnectivity(serverId: string): Promise<ConnectivityResult>;
}

export { createMockClient } from "./mock.ts";
