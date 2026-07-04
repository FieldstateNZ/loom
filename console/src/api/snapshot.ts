import type {
  VirtualKey,
  Tenant,
  CredOverride,
  Provider,
  McpServer,
  Conversation,
} from "./models.ts";
import type {
  GatewayStats,
  UsageDaily,
  BarItem,
  GatewayEvent,
  UsageByKey,
} from "./metrics.ts";
import type { BudgetWindow, BudgetMode } from "./models.ts";

/**
 * Everything the console loads on boot, in one aggregate. The mock returns it
 * from a single seed; the live client maps it from several gateway endpoints
 * (degrading honestly to empty collections where an endpoint is missing). All
 * collections are `readonly` — screens derive views, they never mutate it.
 */
export interface LoomSnapshot {
  readonly now: string;
  readonly keys: readonly VirtualKey[];
  readonly tenants: readonly Tenant[];
  readonly credOverrides: readonly CredOverride[];
  readonly providers: readonly Provider[];
  readonly stats: GatewayStats;
  readonly spendByHour: readonly number[];
  readonly priorByHour: readonly number[];
  readonly usageDaily: UsageDaily;
  readonly topModels: readonly BarItem[];
  readonly topKeys: readonly BarItem[];
  readonly events: readonly GatewayEvent[];
  readonly usageByKey: readonly UsageByKey[];
  readonly mcpServers: readonly McpServer[];
  readonly conversations: readonly Conversation[];
}

/** Payload for issuing a virtual key ({@link LoomClient.createKey}). */
export interface CreateKeyInput {
  readonly name: string;
  readonly tenant: string;
  readonly scopes: readonly string[];
  readonly cap: number | null;
  readonly window: BudgetWindow | null;
  readonly mode: BudgetMode;
}

/** The outcome of a connectivity probe — a bespoke, UI-facing result union. */
export type ConnectivityResult =
  | { readonly ok: true; readonly detail: string }
  | { readonly ok: false; readonly detail: string };
