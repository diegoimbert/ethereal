/**
 * Groove detail tab: the project playback swing (`ProjectSettings::swing`/`swing_grid`),
 * applied to every MIDI clip when the engine plays it; notes are not moved.
 */

import { useRef } from "react";
import { useT } from "@/i18n";
import { NumberField, Select } from "@/kit";
import { useProjectStore } from "@/state";
import { useTransport } from "@/transport";
import { setSwingCommand, SWING_GRIDS, swingGridOf, type SwingGrid } from "./grooveCommands";
import "./groove.css";

const GRID_OPTIONS = SWING_GRIDS.map((g) => ({ value: g.value, label: g.label }));

export function GroovePanel() {
  const t = useT();
  const swing = useProjectStore((s) => s.project?.settings.swing ?? null);
  const swingGrid = useProjectStore((s) => s.project?.settings.swing_grid ?? null);
  if (swing === null || swingGrid === null) {
    return (
      <div className="eth-groove eth-groove--empty" data-feature="groove" data-testid="groove-panel">
        {t("groove.panel.no_project")}
      </div>
    );
  }
  return <SwingControls swing={swing} swingGrid={swingGrid} />;
}

/**
 * The project swing as two compact fields (amount and grid), for the clip inspector's
 * "Groove" row. Nothing without a project.
 */
export function ProjectSwingFields() {
  const swing = useProjectStore((s) => s.project?.settings.swing ?? null);
  const swingGrid = useProjectStore((s) => s.project?.settings.swing_grid ?? null);
  if (swing === null || swingGrid === null) return null;
  return <SwingControls swing={swing} swingGrid={swingGrid} compact />;
}

function SwingControls({ swing, swingGrid, compact = false }: { swing: number; swingGrid: number; compact?: boolean }) {
  const t = useT();
  const transport = useTransport();
  // SetSwing carries both fields: an edit made before the previous one's patch arrives
  // must build on what was sent, not on the (stale) mirror.
  const pending = useRef<{ amount: number; grid: number; inflight: number } | null>(null);
  const set = (change: { amount?: number; grid?: number }) => {
    const base = pending.current ?? { amount: swing, grid: swingGrid, inflight: 0 };
    const next = {
      amount: Math.min(1, Math.max(0, change.amount ?? base.amount)),
      grid: change.grid ?? base.grid,
      inflight: base.inflight + 1,
    };
    pending.current = next;
    transport
      .send(setSwingCommand(next.amount, next.grid))
      .catch((e: unknown) => console.warn("[groove] SetSwing failed:", e))
      .finally(() => {
        const p = pending.current;
        if (p && --p.inflight === 0) pending.current = null;
      });
  };
  if (compact) {
    return (
      <span className="eth-groove__compact" data-feature="groove" title={t("groove.panel.swing_title")}>
        <NumberField
          size="sm"
          aria-label={t("groove.panel.swing_label")}
          value={Math.round(swing * 100)}
          min={0}
          max={100}
          unit="%"
          onChange={(v) => set({ amount: v / 100 })}
        />
        <Select<SwingGrid>
          size="sm"
          aria-label={t("groove.panel.grid_label")}
          options={GRID_OPTIONS}
          value={swingGridOf(swingGrid)}
          onChange={(g) => set({ grid: SWING_GRIDS.find((o) => o.value === g)!.beats })}
        />
      </span>
    );
  }
  return (
    <div className="eth-groove" data-feature="groove" data-testid="groove-panel">
      <label className="eth-groove__field">
        <span className="eth-groove__label">{t("groove.panel.swing")}</span>
        <NumberField
          size="sm"
          aria-label={t("groove.panel.swing_label")}
          value={Math.round(swing * 100)}
          min={0}
          max={100}
          unit="%"
          onChange={(v) => set({ amount: v / 100 })}
        />
      </label>
      <label className="eth-groove__field">
        <span className="eth-groove__label">{t("groove.panel.grid")}</span>
        <Select<SwingGrid>
          size="sm"
          aria-label={t("groove.panel.grid_label")}
          options={GRID_OPTIONS}
          value={swingGridOf(swingGrid)}
          onChange={(g) => set({ grid: SWING_GRIDS.find((o) => o.value === g)!.beats })}
        />
      </label>
      <p className="eth-groove__hint">{t("groove.panel.hint")}</p>
    </div>
  );
}
