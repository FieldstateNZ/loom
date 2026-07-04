// Barrel for the Loom Console design-system components. Screens import from
// here, mirroring the `NS` namespace the prototype used.

// core
export { Icon, ICON_NAMES, type IconName } from "./core/Icon.tsx";
export { Spinner } from "./core/Spinner.tsx";
export { Badge, type BadgeTone } from "./core/Badge.tsx";
export { Button, type ButtonVariant } from "./core/Button.tsx";
export { IconButton } from "./core/IconButton.tsx";
export { Kbd } from "./core/Kbd.tsx";
export { SegmentedControl, type SegmentOption } from "./core/SegmentedControl.tsx";
export { StatusDot, type StatusTone } from "./core/StatusDot.tsx";
export { Card } from "./core/Card.tsx";

// data
export { Sparkline } from "./data/Sparkline.tsx";
export { DeltaTag } from "./data/DeltaTag.tsx";
export { StatTile } from "./data/StatTile.tsx";
export { BudgetBar } from "./data/BudgetBar.tsx";
export { BarList } from "./data/BarList.tsx";
export { FilterChip } from "./data/FilterChip.tsx";
export { LineChart, ChartLegend, type ChartSeries } from "./data/LineChart.tsx";
export { BarChart } from "./data/BarChart.tsx";
export { DataTable, type Column } from "./data/DataTable.tsx";

// forms
export { Field } from "./forms/Field.tsx";
export { Input } from "./forms/Input.tsx";
export { Select } from "./forms/Select.tsx";
export { Switch } from "./forms/Switch.tsx";
export { SecretInput } from "./forms/SecretInput.tsx";

// feedback
export { Banner } from "./feedback/Banner.tsx";
export { Dialog } from "./feedback/Dialog.tsx";
export { EmptyState } from "./feedback/EmptyState.tsx";
export { RevealOnce } from "./feedback/RevealOnce.tsx";

// navigation
export { SideNav, type NavSection, type NavItem } from "./navigation/SideNav.tsx";
export { TopBar, type Crumb } from "./navigation/TopBar.tsx";

// transcript
export { Transcript } from "./transcript/Transcript.tsx";
export { Turn } from "./transcript/Turn.tsx";
export { BlockFrame, JsonPre } from "./transcript/blockBase.tsx";
export { BlockText } from "./transcript/BlockText.tsx";
export { BlockThinking } from "./transcript/BlockThinking.tsx";
export { BlockToolUse } from "./transcript/BlockToolUse.tsx";
export { BlockWebSearch } from "./transcript/BlockWebSearch.tsx";
export { BlockCodeExec } from "./transcript/BlockCodeExec.tsx";
export { BlockUnknown } from "./transcript/BlockUnknown.tsx";
export { CacheMarker } from "./transcript/CacheMarker.tsx";
