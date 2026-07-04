// Core admin-domain models — the tenant/key/provider/MCP shapes the gateway's
// admin + usage REST API returns. Every field is `readonly`: these are DTOs the
// console reads and renders, never mutates in place.

/** Lifecycle state of a virtual key. */
export type KeyStatus = "active" | "blocked" | "revoked";

/** The period a budget cap resets over. `total` never resets (a hard lifetime cap). */
export type BudgetWindow = "daily" | "weekly" | "monthly" | "total";

/** What happens when a budget cap is crossed: refuse requests, or only flag. */
export type BudgetMode = "block" | "warn";

/** A capability a virtual key may be granted. */
export type Scope = "messages" | "streaming" | "mcp";

/** A virtual key: the credential a tenant's product uses to call the gateway. */
export interface VirtualKey {
  readonly id: string;
  readonly name: string;
  readonly tenant: string;
  readonly status: KeyStatus;
  readonly scopes: readonly string[];
  readonly budgetSpent: number;
  /** Cap in USD, or `null` when the key is uncapped (spend still metered). */
  readonly cap: number | null;
  readonly window: BudgetWindow | null;
  readonly mode: BudgetMode;
  /** Human "last used" label (e.g. `"18s ago"`). */
  readonly last: string;
  readonly spend7: number;
  readonly rateRpm?: number;
}

/** A tenant: the isolation boundary that keys, budgets and MCP servers scope to. */
export interface Tenant {
  readonly id: string;
  readonly name: string;
  readonly status: "active" | "suspended";
  readonly keys: number;
  readonly mcp: number;
  readonly spend30: number;
  readonly cap: number | null;
  readonly window: BudgetWindow;
  /** This tenant's fraction of total gateway spend, 0..1. */
  readonly share: number;
  readonly requests30: number;
  readonly blocks30: number;
}

/** A per-tenant provider-credential override (vs. inheriting the gateway default). */
export interface CredOverride {
  readonly tenant: string;
  readonly provider: string;
  readonly set: boolean;
  readonly meta: string | null;
  readonly baseUrl: string | null;
}

/** An upstream model provider (Anthropic is the first, not the only, shape). */
export interface Provider {
  readonly id: string;
  readonly name: string;
  readonly api: "native" | "translated";
  readonly status: "connected" | "error";
  readonly keyMeta: string;
  readonly baseUrl: string | null;
  readonly defaultBaseUrl: string;
  readonly models: number;
  readonly lastCheck: string;
}

/** A registered MCP server a tenant's conversations can reference by name. */
export interface McpServer {
  readonly id: string;
  readonly tenant: string;
  readonly name: string;
  readonly url: string;
  readonly status: "connected" | "error";
  readonly last: string;
  readonly tokenMeta: string;
}

/** A conversation summary row for the conversations list. */
export interface Conversation {
  readonly id: string;
  readonly key: string;
  readonly model: string;
  readonly turns: number;
  readonly last: string;
  readonly cost: number;
  readonly tokens: number;
  readonly preview: string;
  readonly blocked?: boolean;
}
