// LoomProvider — makes the active LoomClient available to any component (used
// by dialogs deep in the tree for mutations/connectivity checks).
import { createContext, useContext, useMemo, type ReactNode } from "react";
import type { LoomClient } from "./client.ts";
import { createMockClient } from "./mock.ts";

const LoomContext = createContext<LoomClient | null>(null);

export function LoomProvider({ client, children }: { client?: LoomClient; children: ReactNode }) {
  const value = useMemo(() => client ?? createMockClient(), [client]);
  return <LoomContext.Provider value={value}>{children}</LoomContext.Provider>;
}

export function useLoom(): LoomClient {
  const client = useContext(LoomContext);
  if (!client) throw new Error("useLoom must be used within a <LoomProvider>");
  return client;
}
