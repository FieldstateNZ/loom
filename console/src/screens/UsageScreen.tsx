// UsageScreen — filterable cost/token explorer with the cache ROI split.
import { useState } from "react";
import {
  Card, LineChart, BarChart, DataTable, FilterChip, Select, Button, StatTile,
  type Column,
} from "../components/index.ts";
import { Fmt } from "../lib/format.ts";
import type { LoomSnapshot, UsageByKey } from "../api/types.ts";

interface Filter { field: string; value: string; }

export interface UsageScreenProps {
  data: LoomSnapshot;
  range: string;
}

export function UsageScreen({ data, range }: UsageScreenProps) {
  const formatMoney = Fmt.money, formatTokens = Fmt.tokens, formatPercent = Fmt.percent;
  const [filters, setFilters] = useState<Filter[]>([{ field: "key", value: "lucidbrain-prod" }]);
  const [groupBy, setGroupBy] = useState("key");
  const u = data.usageDaily;
  const s = data.stats;

  const columns: Column<UsageByKey>[] = [
    { key: "key", label: groupBy, mono: true },
    { key: "requests", label: "Requests", align: "right", mono: true, render: (r) => r.requests.toLocaleString() },
    { key: "input", label: "Input", align: "right", mono: true, render: (r) => formatTokens(r.input) },
    { key: "output", label: "Output", align: "right", mono: true, render: (r) => formatTokens(r.output) },
    { key: "cacheRead", label: "Cache read", align: "right", mono: true, render: (r) => <span style={{ color: "var(--cache-read)" }}>{formatTokens(r.cacheRead)}</span> },
    { key: "cacheWrite", label: "Cache write", align: "right", mono: true, render: (r) => <span style={{ color: "var(--cache-write)" }}>{formatTokens(r.cacheWrite)}</span> },
    { key: "cost", label: "Cost", align: "right", mono: true, render: (r) => formatMoney(r.cost) },
  ];

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "12px" }}>
      <div style={{ display: "flex", gap: "8px", alignItems: "center", flexWrap: "wrap" }}>
        {filters.map((f, i) => (
          <FilterChip key={f.field + f.value} field={f.field} value={f.value}
            onRemove={() => setFilters(filters.filter((_, j) => j !== i))} />
        ))}
        <Select size="sm" label="Add filter" value=""
          options={[{ value: "", label: "+ Add filter" }, { value: "model", label: "model" }, { value: "conversation", label: "conversation" }, { value: "tenant", label: "tenant" }]}
          onChange={(v) => { if (v) setFilters([...filters, { field: v, value: v === "model" ? "claude-sonnet-4-5" : v === "tenant" ? "lucidbrain" : "conv_9f2c4e8a" }]); }} />
        <span style={{ flex: 1 }}></span>
        <span style={{ font: "var(--w-reg) var(--fs-12)/1 var(--font-sans)", color: "var(--fg-3)" }}>Group by</span>
        <Select size="sm" mono options={["key", "model", "tenant", "conversation"]} value={groupBy} onChange={setGroupBy} label="Group by" />
        <Button size="sm" icon="download">Export CSV</Button>
      </div>
      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr", gap: "10px" }}>
        <StatTile label="Cache reads — 7d" value={formatTokens(u.cacheRead.reduce((a, b) => a + b, 0))}
          sub={"≈ " + formatMoney(22.68) + " avoided input spend"} />
        <StatTile label="Cache hit rate" value={formatPercent(s.cacheHitRate)} sub="of input tokens served from cache" />
        <StatTile label="Cache writes — 7d" value={formatTokens(u.cacheWrite.reduce((a, b) => a + b, 0))}
          sub={"premium paid ≈ " + formatMoney(4.1)} />
      </div>
      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: "10px", alignItems: "start" }}>
        <Card eyebrow={"Cost — " + range}>
          <LineChart area height={180}
            series={[{ name: "cost", color: "var(--series-1)", data: u.cost }]}
            yFormat={(v) => formatMoney(v, { compact: true })}
            xLabels={[u.labels[0], "", "", u.labels[3], "", "", u.labels[6]]} />
        </Card>
        <Card eyebrow="Tokens — cache split">
          <BarChart height={180}
            series={[
              { name: "input", color: "var(--bg-3)", data: u.input },
              { name: "output", color: "var(--fg-4)", data: u.output },
              { name: "cache read", color: "var(--cache-read)", data: u.cacheRead },
              { name: "cache write", color: "var(--cache-write)", data: u.cacheWrite },
            ]}
            yFormat={formatTokens}
            xLabels={[u.labels[0], "", "", u.labels[3], "", "", u.labels[6]]}
            titles={u.labels} />
        </Card>
      </div>
      <Card eyebrow={"Usage by " + groupBy + " — 7d"} flush footer={<span>4 of 4 groups · filters apply before grouping</span>}>
        <DataTable
          rowKey="key"
          onRowClick={() => {}}
          columns={columns}
          rows={data.usageByKey}
        />
      </Card>
    </div>
  );
}
