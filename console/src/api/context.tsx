// LoomProvider — makes the active LoomClient available to any component (used
// by dialogs deep in the tree for mutations/connectivity checks).
import { createContext, useContext, useMemo, type ReactNode } from "react";
import type { LoomClient } from "./client.ts";
import { createMockClient } from "./mock.ts";
import { createHttpClient } from "./http.ts";

const LoomContext = createContext<LoomClient | null>(null);

/** The resolved live-gateway configuration and whether it is active. */
export interface LoomConfig {
  /** True when a live base URL is configured (env / ?api= / localStorage). */
  live: boolean;
  baseUrl?: string;
  adminToken?: string;
  apiKey?: string;
}

function firstNonEmpty(...vals: (string | undefined | null)[]): string | undefined {
  for (const v of vals) {
    if (typeof v === "string" && v.trim() !== "") return v.trim();
  }
  return undefined;
}

function fromLocalStorage(key: string): string | undefined {
  try {
    return window.localStorage.getItem(key) ?? undefined;
  } catch {
    return undefined; // storage may be unavailable (sandboxed/SSR)
  }
}

/**
 * Resolves the live-gateway configuration from, in precedence order:
 *   1. URL param `?api=<baseUrl>` (base URL only — never tokens, to keep secrets
 *      out of the address bar / history)
 *   2. localStorage: `loom.baseUrl` / `loom.adminToken` / `loom.apiKey`
 *   3. Vite env: `VITE_LOOM_BASE_URL` / `VITE_LOOM_ADMIN_TOKEN` / `VITE_LOOM_API_KEY`
 *
 * `live` is true only when a base URL resolves; otherwise the mock is used.
 */
export function resolveLoomConfig(): LoomConfig {
  const env = import.meta.env;
  let apiParam: string | undefined;
  try {
    apiParam = new URLSearchParams(window.location.search).get("api") ?? undefined;
  } catch {
    apiParam = undefined;
  }

  const baseUrl = firstNonEmpty(apiParam, fromLocalStorage("loom.baseUrl"), env.VITE_LOOM_BASE_URL);
  const adminToken = firstNonEmpty(fromLocalStorage("loom.adminToken"), env.VITE_LOOM_ADMIN_TOKEN);
  const apiKey = firstNonEmpty(fromLocalStorage("loom.apiKey"), env.VITE_LOOM_API_KEY);

  return { live: Boolean(baseUrl), baseUrl, adminToken, apiKey };
}

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

export function LoomProvider({ client, children }: { client?: LoomClient; children: ReactNode }) {
  const value = useMemo(() => client ?? resolveLoomClient(), [client]);
  return <LoomContext.Provider value={value}>{children}</LoomContext.Provider>;
}

export function useLoom(): LoomClient {
  const client = useContext(LoomContext);
  if (!client) throw new Error("useLoom must be used within a <LoomProvider>");
  return client;
}
