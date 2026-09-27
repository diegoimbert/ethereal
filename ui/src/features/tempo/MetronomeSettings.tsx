/**
 * Metronome settings in the top bar (`data-slot="metronome"`): a small button opening a
 * popover with the on/off switch (`Transport::SetMetronome`), click volume, accent and
 * sound (`Tempo::SetMetronomeSettings`, one undo step each). The click also sounds during
 * a recording count-in, even when the metronome is off.
 */

import type { MetronomeSound } from "@/generated";
import { Button, NumberField, Popover, Select, Toggle } from "@/kit";
import { useProjectStore } from "@/state";
import { cmd } from "@/transport";
import { sendEdit, useTempoTransport } from "./gesture";
import { MAX_METRONOME_DB, METRONOME_SOUNDS, metronomeSettingsCommand, MIN_METRONOME_DB } from "./tempoCommands";
import "./tempo.css";

export function MetronomeSettings() {
  const settings = useProjectStore((s) => s.project?.settings ?? null);
  const transport = useTempoTransport();
  if (!settings) return null;
  const on = settings.metronome;
  return (
    <Popover
      aria-label="Metronome settings"
      placement="bottom-start"
      trigger={(props) => (
        <Button
          {...props}
          size="sm"
          tone="ghost"
          aria-label="Metronome settings"
          title="Metronome settings (volume, accent, sound)"
          data-testid="metronome-settings"
        >
          ▾
        </Button>
      )}
    >
      <div className="eth-metronome" data-feature="metronome">
        <Toggle
          size="sm"
          label="Metronome"
          checked={on}
          onChange={(enabled) => void sendEdit(transport, cmd("Transport", { type: "SetMetronome", enabled }))}
        />
        <label className="eth-metronome__field">
          <span className="eth-metronome__label">Volume</span>
          <NumberField
            size="sm"
            aria-label="Metronome volume"
            value={Math.max(MIN_METRONOME_DB, settings.metronome_volume)}
            min={MIN_METRONOME_DB}
            max={MAX_METRONOME_DB}
            precision={1}
            unit="dB"
            onChange={(volume) => void sendEdit(transport, metronomeSettingsCommand({ volume }))}
          />
        </label>
        <Toggle
          size="sm"
          label="Accent downbeat"
          checked={settings.metronome_accent}
          onChange={(accent) => void sendEdit(transport, metronomeSettingsCommand({ accent }))}
        />
        <label className="eth-metronome__field">
          <span className="eth-metronome__label">Sound</span>
          <Select<MetronomeSound>
            size="sm"
            aria-label="Metronome sound"
            options={METRONOME_SOUNDS}
            value={settings.metronome_sound}
            onChange={(sound) => void sendEdit(transport, metronomeSettingsCommand({ sound }))}
          />
        </label>
        <p className="eth-metronome__hint">The click also counts in before recording, even when off.</p>
      </div>
    </Popover>
  );
}
