/**
 * Groove detail tab: the project playback swing (`ProjectSettings::swing`/`swing_grid`),
 * applied to every MIDI clip when the engine plays it; notes are not moved.
 */

import { NumberField, Select } from "@/kit";
import { useProjectStore } from "@/state";
import { useTransport } from "@/transport";
import { setSwingCommand, SWING_GRIDS, swingGridOf, type SwingGrid } from "./grooveCommands";
import "./groove.css";

const GRID_OPTIONS = SWING_GRIDS.map((g) => ({ value: g.value, label: g.label }));

export function GroovePanel() {
  const transport = useTransport();
  const swing = useProjectStore((s) => s.project?.settings.swing ?? null);
  const swingGrid = useProjectStore((s) => s.project?.settings.swing_grid ?? null);
  if (swing === null || swingGrid === null) {
    return (
      <div className="eth-groove eth-groove--empty" data-testid="groove-panel">
        No project open.
      </div>
    );
  }
  const set = (amount: number, grid: number) => {
    transport.send(setSwingCommand(amount, grid)).catch((e: unknown) => console.warn("[groove] SetSwing failed:", e));
  };
  return (
    <div className="eth-groove" data-testid="groove-panel">
      <label className="eth-groove__field">
        <span className="eth-groove__label">Swing</span>
        <NumberField
          size="sm"
          aria-label="Project swing"
          value={Math.round(swing * 100)}
          min={0}
          max={100}
          unit="%"
          onChange={(v) => set(v / 100, swingGrid)}
        />
      </label>
      <label className="eth-groove__field">
        <span className="eth-groove__label">Grid</span>
        <Select<SwingGrid>
          size="sm"
          aria-label="Swing grid"
          options={GRID_OPTIONS}
          value={swingGridOf(swingGrid)}
          onChange={(g) => set(swing, SWING_GRIDS.find((o) => o.value === g)!.beats)}
        />
      </label>
      <p className="eth-groove__hint">
        Playback swing delays notes on off-beat grid positions for every MIDI clip. Notes are not moved; use Quantize
        with swing in the piano roll to write it into the clip.
      </p>
    </div>
  );
}
