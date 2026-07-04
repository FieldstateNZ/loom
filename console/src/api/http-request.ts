// The live gateway's HTTP transport. `createRequest` returns a bound fetch that
// yields a Result — expected failures (non-2xx, network errors, unparseable
// bodies) come back as `err(...)` rather than throwing, so callers branch on
// `.ok`. A 404 with `allow404` and a 204 both resolve to `ok(null)`.
import { asRecord, str } from "./json.ts";
import { ok, err, loomError, type Result, type LoomError } from "./result.ts";
import { rowsOf, toUsageRow, type UsageRow } from "./usage.ts";

/** Per-request options. */
export interface RequestOptions {
  readonly method?: string;
  readonly token?: string | undefined;
  readonly body?: unknown;
  /** When true, a 404 resolves to `ok(null)` instead of an error. */
  readonly allow404?: boolean;
}

/** A fetch bound to a base URL, returning a {@link Result} (never throwing). */
export type RequestFn = <T = unknown>(
  path: string,
  init?: RequestOptions,
) => Promise<Result<T | null, LoomError>>;

/** Builds a {@link RequestFn} bound to `baseUrl` (trailing slashes trimmed). */
export function createRequest(baseUrl: string): RequestFn {
  const root = baseUrl.replace(/\/+$/, "");
  return async function request<T = unknown>(path: string, init: RequestOptions = {}) {
    const headers: Record<string, string> = { accept: "application/json" };
    if (init.token) headers.authorization = `Bearer ${init.token}`;
    if (init.body !== undefined) headers["content-type"] = "application/json";

    let res: Response;
    try {
      res = await fetch(`${root}${path}`, {
        method: init.method ?? "GET",
        headers,
        ...(init.body !== undefined ? { body: JSON.stringify(init.body) } : {}),
      });
    } catch (e) {
      return err(loomError(e instanceof Error ? e.message : "network request failed", "network"));
    }

    if ((res.status === 404 && init.allow404) || res.status === 204) return ok(null);
    if (!res.ok) return err(loomError(await errorDetail(res), "http", res.status));
    try {
      return ok((await res.json()) as T);
    } catch {
      return err(loomError("response body was not valid JSON", "parse", res.status));
    }
  };
}

/** Pulls the most specific error message out of a failed response body. */
async function errorDetail(res: Response): Promise<string> {
  try {
    const rec = asRecord(await res.json());
    const msg = rec && (str(rec.message) ?? str((asRecord(rec.error) ?? {}).message) ?? str(rec.error));
    if (msg) return msg;
  } catch {
    /* non-JSON error body */
  }
  return `${res.status} ${res.statusText}`;
}

/** GETs a usage rollup and validates its rows; returns `[]` on any failure. */
export async function usageRollup(
  request: RequestFn,
  path: string,
  token: string | undefined,
  params: Record<string, string>,
): Promise<UsageRow[]> {
  if (!token) return [];
  const qs = new URLSearchParams(params).toString();
  const res = await request<unknown>(`${path}?${qs}`, { token });
  if (!res.ok || res.value == null) return [];
  return rowsOf(res.value).map(toUsageRow);
}
