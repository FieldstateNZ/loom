// BudgetEditDialog — edit a cap/window/mode/rate-limit for a key or a tenant.
import { useState } from "react";
import { Button, Dialog, Field, Input, Select, Switch } from "../components/index.ts";
import type { BudgetWindow } from "../api/types.ts";

/** The thing being budgeted — a key row, or a tenant cast into this shape. */
export interface BudgetSubject {
  readonly name: string;
  readonly cap?: number | null;
  readonly window?: string | null;
  readonly mode?: "block" | "warn";
  readonly rateRpm?: number;
}

/** Props for {@link BudgetEditDialog}. */
export interface BudgetEditDialogProps {
  /** The subject to edit; `null` closes the dialog. */
  readonly subject: BudgetSubject | null;
  readonly onClose: () => void;
}

/** Modal for editing a subject's spend cap, window, over-cap behaviour and rate limit. */
export function BudgetEditDialog({ subject, onClose }: BudgetEditDialogProps) {
  const [cap, setCap] = useState(subject && subject.cap != null ? String(subject.cap) : "");
  const [win, setWin] = useState<BudgetWindow>(((subject && subject.window) as BudgetWindow) || "daily");
  const [block, setBlock] = useState(subject ? subject.mode === "block" : true);
  const [rpm, setRpm] = useState(subject && subject.rateRpm ? String(subject.rateRpm) : "60");
  if (!subject) return null;
  return (
    <Dialog open onClose={onClose} title={"Budget — " + subject.name} icon="wallet" width={460}
      footer={<>
        <Button variant="ghost" onClick={onClose}>Cancel</Button>
        <Button variant="primary" onClick={onClose}>Save budget</Button>
      </>}>
      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: "14px" }}>
        <Field label="Cap (USD)" hint="Leave empty for no cap — spend is still metered.">
          <Input mono value={cap} onChange={setCap} placeholder="no cap" />
        </Field>
        <Field label="Window">
          <Select options={["daily", "weekly", "monthly", "total"]} value={win} onChange={(v) => setWin(v as BudgetWindow)} />
        </Field>
        <div style={{ gridColumn: "1 / -1" }}>
          <Field hint={block ? "Requests over the cap are refused with 402 budget_exceeded." : "Over-cap requests continue; the key is flagged on the dashboard."}>
            <Switch checked={block} onChange={setBlock} label="Block requests over cap" />
          </Field>
        </div>
        <Field label="Rate limit (requests/min)">
          <Input mono value={rpm} onChange={setRpm} />
        </Field>
        <Field label="Applies">
          <Input readOnly mono value={win === "total" ? "until raised" : "resets 00:00 UTC"} />
        </Field>
      </div>
    </Dialog>
  );
}
