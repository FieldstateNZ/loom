// Resolves the live-gateway configuration the console runs against, from Vite
// env, a `?api=` URL param, or localStorage. Tokens are deliberately never read
// from the URL (they would leak into history/logs) — only the base URL is.

/** The resolved live-gateway configuration and whether it is active. */
export interface LoomConfig {
  /** True when a live base URL is configured (env / ?api= / localStorage). */
  readonly live: boolean;
  readonly baseUrl?: string | undefined;
  readonly adminToken?: string | undefined;
  readonly apiKey?: string | undefined;
}

/** Returns the first non-blank, trimmed value, or `undefined`. */
function firstNonEmpty(...vals: (string | undefined | null)[]): string | undefined {
  for (const v of vals) {
    if (typeof v === "string" && v.trim() !== "") return v.trim();
  }
  return undefined;
}

/** Reads a localStorage key, tolerating environments where storage is unavailable. */
function fromLocalStorage(key: string): string | undefined {
  try {
    return window.localStorage.getItem(key) ?? undefined;
  } catch {
    return undefined; // storage may be unavailable (sandboxed/SSR)
  }
}

/**
 * Resolves the live-gateway configuration, in precedence order:
 *   1. URL param `?api=<baseUrl>` (base URL only — never tokens)
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
