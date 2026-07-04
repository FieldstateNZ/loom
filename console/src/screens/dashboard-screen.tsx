// DashboardScreen — the money question, answered in one glance.
import {
  Card, StatTile, BarList, LineChart, DataTable, Badge, Banner, Button, StatusDot,
  type Column,
} from "../components/index.ts";
import { Fmt } from "../lib/format.ts";
import type { LoomSnapshot, GatewayEvent } from "../api/types.ts";

/** Props for {@link DashboardScreen}. */
export interface DashboardScreenProps {
  readonly data: LoomSnapshot;
  /** The selected time range label (drives the tile/label copy). */
  readonly range: string;
  readonly role: "operator" | "tenant";
  readonly tenant: string;
}

/** The overview screen: hero spend, token/request tiles, spend chart, top-N, events. */
export function DashboardScreen({ data, range, role, tenant }: DashboardScreenProps) {
  const formatMoney = Fmt.money, formatTokens = Fmt.tokens;
  const s = data.stats;
  const scoped = role === "tenant";
  const eventColumns: Column<GatewayEvent>[] = [
    { key: "time", label: "Time", width: "70px", mono: true, muted: true },
    { key: "kind", label: "Event", width: "110px", render: (r) => <Badge tone={r.tone} caps>{r.kind}</Badge> },
    { key: "key", label: "Key", mono: true },
    { key: "detail", label: "Detail", muted: true },
  ];
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "12px" }}>
      <Banner tone="danger" title="Budget block active"
        action={<Button size="sm">Review budget</Button>}>
        atrium-staging hit its $20 daily cap at 14:32 NZST. Requests are being refused until the window resets.
      </Banner>
      <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(180px, 1fr))", gap: "10px" }}>
        <StatTile hero label={"Spend — " + (scoped ? tenant : "gateway")} labelRight={range}
          value={formatMoney(scoped ? s.spendToday * 0.58 : s.spendToday)} delta={s.spendDelta} invertDelta
          sub={"vs " + formatMoney(s.spendPrior) + " prior · resets 00:00 UTC"} spark={data.spendByHour}
          style={{ gridColumn: "span 2" }} />
        <StatTile label="Tokens in" value={formatTokens(s.tokensIn)} delta={s.tokensInDelta} spark={data.spendByHour.map((v, i) => v + (i % 3))} sparkColor="var(--series-2)" />
        <StatTile label="Tokens out" value={formatTokens(s.tokensOut)} delta={s.tokensOutDelta} spark={data.priorByHour} sparkColor="var(--series-2)" />
        <StatTile label="Requests" value={s.requests.toLocaleString()} delta={s.requestsDelta} spark={data.spendByHour.slice().reverse()} sparkColor="var(--series-4)" />
        <StatTile label="Active streams" value={String(s.streams)}
          sub={<StatusDot tone="accent" pulse label="live via SSE" />} />
      </div>
      <div style={{ display: "grid", gridTemplateColumns: "1.6fr 1fr", gap: "10px", alignItems: "start" }}>
        <Card eyebrow={"Spend — " + range}
          actions={<span style={{ font: "var(--w-reg) var(--fs-11)/1 var(--font-mono)", color: "var(--fg-3)" }}>{data.now}</span>}>
          <LineChart area height={190}
            series={[
              { name: "this period", color: "var(--series-1)", data: data.spendByHour },
              { name: "prior period", color: "var(--fg-4)", data: data.priorByHour },
            ]}
            yFormat={(v) => formatMoney(v)}
            xLabels={["00:00", "06:00", "12:00", "18:00", "now"]} />
        </Card>
        <div style={{ display: "flex", flexDirection: "column", gap: "10px" }}>
          <Card eyebrow="Top models — spend" flush>
            <div style={{ padding: "10px 12px 12px" }}>
              <BarList mono items={data.topModels} />
            </div>
          </Card>
          <Card eyebrow="Top keys — spend" flush>
            <div style={{ padding: "10px 12px 12px" }}>
              <BarList mono items={data.topKeys} color="var(--series-1)" />
            </div>
          </Card>
        </div>
      </div>
      <Card eyebrow="Recent blocks & errors" flush footer={<span>Streaming from gateway events · last 4h</span>}>
        <DataTable
          dense
          rowKey={(r) => r.time + r.key}
          columns={eventColumns}
          rows={data.events}
        />
      </Card>
    </div>
  );
}
