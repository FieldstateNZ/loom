// App — the Loom Console shell. This is the real implementation of the design
// bundle's templates/console-screen/ConsoleScreen.dc.html: a role-scoped
// SideNav + TopBar wrapping a content area that swaps between all eight
// designed screens, with tenant/provider drill-in detail.
//
// Deep-linkable: ?screen=keys&role=tenant&tenant=lucidbrain&theme=light —
// state syncs to the URL so a refresh keeps your place.
import { useEffect, useMemo, useState, type ReactNode } from "react";
import {
  SideNav, TopBar, SegmentedControl, Badge, StatusDot, Icon, Spinner,
  type NavSection, type Crumb,
} from "./components/index.ts";
import { LoomProvider } from "./api/context.tsx";
import { createMockClient } from "./api/mock.ts";
import type { LoomClient } from "./api/client.ts";
import type { LoomSnapshot } from "./api/types.ts";

import { DashboardScreen } from "./screens/DashboardScreen.tsx";
import { KeysScreen } from "./screens/KeysScreen.tsx";
import { UsageScreen } from "./screens/UsageScreen.tsx";
import { ConversationsScreen } from "./screens/ConversationsScreen.tsx";
import { TenantsScreen } from "./screens/TenantsScreen.tsx";
import { BudgetsScreen } from "./screens/BudgetsScreen.tsx";
import { McpScreen } from "./screens/McpScreen.tsx";
import { ProvidersScreen } from "./screens/ProvidersScreen.tsx";

type ScreenId =
  | "overview" | "usage" | "conversations" | "keys"
  | "budgets" | "mcp" | "tenants" | "credentials";
type Role = "operator" | "tenant";
type Theme = "dark" | "light";

const SCREEN_TITLES: Record<ScreenId, string> = {
  overview: "Overview", usage: "Usage explorer", conversations: "Conversations",
  keys: "Keys", budgets: "Budgets & limits", mcp: "MCP servers",
  tenants: "Tenants", credentials: "Provider credentials",
};

function isScreenId(v: string | null): v is ScreenId {
  return !!v && Object.prototype.hasOwnProperty.call(SCREEN_TITLES, v);
}

function Console({ client }: { client: LoomClient }) {
  const params = new URLSearchParams(window.location.search);
  const initialScreen = params.get("screen");
  const [screen, setScreen] = useState<ScreenId>(isScreenId(initialScreen) ? initialScreen : "overview");
  const [range, setRange] = useState("24h");
  const [theme, setTheme] = useState<Theme>(params.get("theme") === "light" ? "light" : "dark");
  const [role, setRole] = useState<Role>(params.get("role") === "tenant" ? "tenant" : "operator");
  const [tenantDetail, setTenantDetail] = useState<string | null>(params.get("tenant"));
  const [providerDetail, setProviderDetail] = useState<string | null>(params.get("provider"));
  const [data, setData] = useState<LoomSnapshot | null>(null);
  const tenant = "lucidbrain";

  useEffect(() => {
    let live = true;
    client.bootstrap().then((snap) => { if (live) setData(snap); });
    return () => { live = false; };
  }, [client]);

  useEffect(() => { document.documentElement.dataset.theme = theme; }, [theme]);

  useEffect(() => {
    // Preserve host params — only manage our own keys.
    const p = new URLSearchParams(window.location.search);
    (["screen", "role", "tenant", "provider", "theme"] as const).forEach((k) => p.delete(k));
    if (screen !== "overview") p.set("screen", screen);
    if (role !== "operator") p.set("role", role);
    if (tenantDetail) p.set("tenant", tenantDetail);
    if (providerDetail) p.set("provider", providerDetail);
    if (theme !== "dark") p.set("theme", theme);
    const qs = p.toString();
    try { window.history.replaceState(null, "", qs ? "?" + qs : window.location.pathname); } catch { /* sandboxed hosts may refuse */ }
  }, [screen, role, tenantDetail, providerDetail, theme]);

  const go = (id: string) => { setTenantDetail(null); setProviderDetail(null); setScreen(id as ScreenId); };

  const sections: NavSection[] = [
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

  const screenEl: ReactNode = !data ? null : {
    overview: <DashboardScreen data={data} range={range} role={role} tenant={tenant} />,
    keys: <KeysScreen data={data} role={role} tenant={tenant} />,
    usage: <UsageScreen data={data} range={range === "24h" ? "7d" : range} />,
    conversations: <ConversationsScreen data={data} role={role} tenant={tenant} />,
    tenants: <TenantsScreen data={data} detailId={tenantDetail} onOpenTenant={(t) => setTenantDetail(t.id)} />,
    budgets: <BudgetsScreen data={data} role={role} tenant={tenant} />,
    mcp: <McpScreen data={data} role={role} tenant={tenant} />,
    credentials: <ProvidersScreen data={data} detailId={providerDetail} onOpenProvider={(p) => setProviderDetail(p.id)} />,
  }[screen];

  const showRange = screen === "overview" || screen === "usage";
  const detailTenant = tenantDetail && data ? data.tenants.find((t) => t.id === tenantDetail) : null;
  const detailProvider = providerDetail && data ? data.providers.find((p) => p.id === providerDetail) : null;
  const crumbs: (string | Crumb)[] = screen === "tenants" && detailTenant
    ? [{ label: "Tenants", onClick: () => setTenantDetail(null) }, detailTenant.name]
    : screen === "credentials" && detailProvider
    ? [{ label: "Provider credentials", onClick: () => setProviderDetail(null) }, detailProvider.name]
    : role === "operator" ? [SCREEN_TITLES[screen]] : ["LucidBrain", SCREEN_TITLES[screen]];

  return (
    <div style={{ display: "flex", height: "100vh", background: "var(--bg-0)", color: "var(--fg-1)", overflow: "hidden", font: "400 13px/1.35 var(--font-sans)" }}>
      <SideNav
        activeId={screen}
        onSelect={go}
        context={
          <button
            type="button"
            onClick={() => { setRole(role === "operator" ? "tenant" : "operator"); setTenantDetail(null); setProviderDetail(null); setScreen("overview"); }}
            title="Demo: switch between operator and tenant-admin scope"
            style={{
              display: "flex", alignItems: "center", gap: "8px", width: "100%", cursor: "pointer",
              padding: "7px 9px", border: "1px solid var(--border-1)", borderRadius: "var(--r-sm)",
              background: "var(--bg-1)", textAlign: "left",
            }}>
            <StatusDot tone="ok" />
            <span style={{ font: "var(--w-med) var(--fs-12)/1 var(--font-sans)", color: "var(--fg-1)", flex: 1 }}>
              {role === "operator" ? "All tenants" : "LucidBrain"}
            </span>
            <Badge tone="info" caps>{role === "operator" ? "Operator" : "Tenant"}</Badge>
            <Icon name="chevron-down" size={13} color="var(--fg-4)" />
          </button>
        }
        sections={sections}
        footer={
          <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: "8px" }}>
            <SegmentedControl label="Theme" value={theme} onChange={(v) => setTheme(v as Theme)}
              options={[{ value: "dark", icon: "moon" }, { value: "light", icon: "sun" }]} />
            <span style={{ font: "var(--w-reg) var(--fs-11)/1 var(--font-mono)", color: "var(--fg-4)" }}>loom v0.4.1</span>
          </div>
        }
      />
      <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column" }}>
        <TopBar
          crumbs={crumbs}
          actions={
            <>
              {showRange ? <SegmentedControl mono label="Time range" options={["24h", "7d", "30d"]} value={range} onChange={setRange} /> : null}
              <StatusDot tone="ok" pulse label="gateway healthy" />
            </>
          }
        />
        <main style={{ flex: 1, overflowY: "auto" }}>
          <div style={{ maxWidth: "var(--page-max-w)", padding: "var(--page-pad)", margin: "0 auto" }} data-screen-label={SCREEN_TITLES[screen]}>
            {data ? screenEl : (
              <div style={{ display: "flex", alignItems: "center", justifyContent: "center", gap: "8px", padding: "80px 0", color: "var(--fg-3)" }}>
                <Spinner size={16} /> Loading gateway…
              </div>
            )}
          </div>
        </main>
      </div>
    </div>
  );
}

export function App() {
  const client = useMemo(() => createMockClient(), []);
  return (
    <LoomProvider client={client}>
      <Console client={client} />
    </LoomProvider>
  );
}
