// Barrel for the Loom Console design-system components. Screens import from
// here, mirroring the `NS` namespace the prototype used.

// core
export { Icon, ICON_NAMES, type IconName } from "./core/icon.tsx";
export { Spinner } from "./core/spinner.tsx";
export { Badge, type BadgeTone } from "./core/badge.tsx";
export { Button, type ButtonVariant } from "./core/button.tsx";
export { IconButton } from "./core/icon-button.tsx";
export { Kbd } from "./core/kbd.tsx";
export { SegmentedControl, type SegmentOption } from "./core/segmented-control.tsx";
export { StatusDot, type StatusTone } from "./core/status-dot.tsx";
export { Card } from "./core/card.tsx";

// data
export { Sparkline } from "./data/sparkline.tsx";
export { DeltaTag } from "./data/delta-tag.tsx";
export { StatTile } from "./data/stat-tile.tsx";
export { BudgetBar } from "./data/budget-bar.tsx";
export { BarList } from "./data/bar-list.tsx";
export { FilterChip } from "./data/filter-chip.tsx";
export { LineChart, ChartLegend, type ChartSeries } from "./data/line-chart.tsx";
export { BarChart } from "./data/bar-chart.tsx";
export { DataTable, type Column } from "./data/data-table.tsx";

// forms
export { Field } from "./forms/field.tsx";
export { Input } from "./forms/input.tsx";
export { Select } from "./forms/select.tsx";
export { Switch } from "./forms/switch.tsx";
export { SecretInput } from "./forms/secret-input.tsx";

// feedback
export { Banner } from "./feedback/banner.tsx";
export { Dialog } from "./feedback/dialog.tsx";
export { EmptyState } from "./feedback/empty-state.tsx";
export { RevealOnce } from "./feedback/reveal-once.tsx";

// navigation
export { SideNav, type NavSection, type NavItem } from "./navigation/side-nav.tsx";
export { TopBar, type Crumb } from "./navigation/top-bar.tsx";

// transcript
export { Transcript } from "./transcript/transcript.tsx";
export { Turn } from "./transcript/turn.tsx";
export { BlockFrame, JsonPre } from "./transcript/block-base.tsx";
export { BlockText } from "./transcript/block-text.tsx";
export { BlockThinking } from "./transcript/block-thinking.tsx";
export { BlockToolUse } from "./transcript/block-tool-use.tsx";
export { BlockWebSearch } from "./transcript/block-web-search.tsx";
export { BlockCodeExec } from "./transcript/block-code-exec.tsx";
export { BlockUnknown } from "./transcript/block-unknown.tsx";
export { CacheMarker } from "./transcript/cache-marker.tsx";
