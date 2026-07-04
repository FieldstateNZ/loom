// LoomProvider — makes the active LoomClient available to any component (used
// by dialogs deep in the tree for mutations/connectivity checks), and selects
// the live HTTP client vs. the frozen mock based on the resolved config.
import { createContext, useContext, useMemo, type ReactNode } from "react";
import { createMockClient } from "./mock-client.ts";
import { createHttpClient } from "./http-client.ts";
import { resolveLoomConfig } from "./config.ts";
import type { LoomClient } from "./client.ts";

const LoomContext = createContext<LoomClient | null>(null);

/**
 * Selects the LoomClient the app runs against: the live HTTP client when a base
 * URL is configured, otherwise the frozen mock (the design/dev default).
 */
export function resolveLoomClient(): LoomClient {
  const cfg = resolveLoomConfig();
  if (cfg.live && cfg.baseUrl) {
    return createHttpClient({ baseUrl: cfg.baseUrl, adminToken: cfg.adminToken, apiKey: cfg.apiKey });
  }
  return createMockClient();
}

/** Props for {@link LoomProvider}. */
export interface LoomProviderProps {
  /** An explicit client (tests/stories); defaults to {@link resolveLoomClient}. */
  readonly client?: LoomClient | undefined;
  readonly children: ReactNode;
}

/** Provides a {@link LoomClient} to the tree via context. */
export function LoomProvider({ client, children }: LoomProviderProps) {
  const value = useMemo(() => client ?? resolveLoomClient(), [client]);
  return <LoomContext.Provider value={value}>{children}</LoomContext.Provider>;
}

/** Reads the active {@link LoomClient}; throws if used outside a provider. */
export function useLoom(): LoomClient {
  const client = useContext(LoomContext);
  if (!client) throw new Error("useLoom must be used within a <LoomProvider>");
  return client;
}
