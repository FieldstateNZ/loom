// LoomClient — the console's view of Loom's admin + usage REST API.
//
// The console codes exclusively against this interface. `createMockClient()`
// (mock-client.ts) satisfies it from a frozen seed for design/dev;
// `createHttpClient()` (http-client.ts) satisfies the same interface against a
// running gateway. Reads that always degrade (bootstrap, getTranscript) return
// their value directly; mutations that can fail return a {@link Result} so the
// UI can branch on `.ok` instead of catching thrown errors.
import type { LoomSnapshot, CreateKeyInput, ConnectivityResult } from "./snapshot.ts";
import type { Transcript } from "./transcript.ts";
import type { VirtualKey } from "./models.ts";
import type { Result, LoomError } from "./result.ts";

/** The one seam between the console and a Loom gateway (live or mocked). */
export interface LoomClient {
  /**
   * GETs the aggregate dashboard/collection snapshot the console boots from.
   * Degrades to empty collections rather than failing, so it returns the value
   * directly.
   */
  bootstrap(): Promise<LoomSnapshot>;

  /** GETs a conversation's turn-by-turn transcript, or `null` if absent. */
  getTranscript(conversationId: string): Promise<Transcript | null>;

  /**
   * Issues a virtual key. On success the plaintext secret is returned exactly
   * once (the shown-once moment). Expected failures (missing admin token, a
   * gateway rejection) come back as `err(...)` for the UI to surface.
   */
  createKey(input: CreateKeyInput): Promise<Result<{ key: VirtualKey; secret: string }, LoomError>>;

  /** Revokes a key. Returns the updated record, or `err(...)` on failure. */
  revokeKey(id: string): Promise<Result<VirtualKey, LoomError>>;

  /** Probes a provider credential's connectivity. */
  checkProviderConnectivity(providerId: string): Promise<ConnectivityResult>;

  /** Probes an MCP server's `tools/list` reachability. */
  checkMcpConnectivity(serverId: string): Promise<ConnectivityResult>;
}
