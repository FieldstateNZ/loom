// Static navigation model for the console shell — screen ids, their titles, and
// the sidenav section builder. Kept separate from the shell component so the
// pure mapping is easy to read and reuse.
import type { NavSection } from "./components/index.ts";

/** The eight console screens, keyed by their URL id. */
export type ScreenId =
  | "overview" | "usage" | "conversations" | "keys"
  | "budgets" | "mcp" | "tenants" | "credentials";

/** Whether the console is scoped to the whole gateway or a single tenant. */
export type Role = "operator" | "tenant";

/** The active colour theme. */
export type Theme = "dark" | "light";

/** Human titles for each screen (used in breadcrumbs and the page label). */
export const SCREEN_TITLES: Record<ScreenId, string> = {
  overview: "Overview", usage: "Usage explorer", conversations: "Conversations",
  keys: "Keys", budgets: "Budgets & limits", mcp: "MCP servers",
  tenants: "Tenants", credentials: "Provider credentials",
};

/** Type guard for a URL `?screen=` value against the known screen ids. */
export function isScreenId(v: string | null): v is ScreenId {
  return !!v && Object.prototype.hasOwnProperty.call(SCREEN_TITLES, v);
}

/** Builds the sidenav sections; the Gateway group only appears for operators. */
export function buildNavSections(role: Role): NavSection[] {
  return [
    { items: [
      { id: "overview", icon: "gauge", label: "Overview" },
      { id: "usage", icon: "chart-line", label: "Usage explorer" },
      { id: "conversations", icon: "message-square", label: "Conversations" },
    ] },
    { label: "Access", items: [
      { id: "keys", icon: "key", label: "Keys" },
      { id: "budgets", icon: "wallet", label: "Budgets & limits", count: 1, tone: "danger" },
      { id: "mcp", icon: "server", label: "MCP servers" },
    ] },
    ...(role === "operator" ? [{ label: "Gateway", items: [
      { id: "tenants", icon: "users", label: "Tenants" },
      { id: "credentials", icon: "shield", label: "Provider credentials" },
    ] } as NavSection] : []),
  ];
}
