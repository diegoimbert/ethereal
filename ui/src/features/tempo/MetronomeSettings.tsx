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
import { useT } from "@/i18n";
import "./tempo.css";

export function MetronomeSettings() {
  const t = useT();
  const settings = useProjectStore((s) => s.project?.settings ?? null);
  const transport = useTempoTransport();
  if (!settings) return null;
  const on = settings.metronome;
  return (
    <Popover
      aria-label={t("tempo.metronome.settings")}
      placement="bottom-start"
      trigger={(props) => (
        <Button
          {...props}
          size="sm"
          tone="ghost"
          aria-label={t("tempo.metronome.settings")}
          title={t("tempo.metronome.settings_title")}
          data-testid="metronome-settings"
        >
          ▾
        </Button>
      )}
    >
      <div className="eth-metronome" data-feature="metronome">
        <Toggle
          size="sm"
          label={t("tempo.metronome.label")}
          checked={on}
          onChange={(enabled) => void sendEdit(transport, cmd("Transport", { type: "SetMetronome", enabled }))}
        />
        <label className="eth-metronome__field">
          <span className="eth-metronome__label">{t("tempo.metronome.volume")}</span>
          <NumberField
            size="sm"
            aria-label={t("tempo.metronome.volume_label")}
            value={Math.max(MIN_METRONOME_DB, settings.metronome_volume)}
            min={MIN_METRONOME_DB}
            max={MAX_METRONOME_DB}
            precision={1}
            unit="db"
            onChange={(volume) => void sendEdit(transport, metronomeSettingsCommand({ volume }))}
          />
        </label>
        <Toggle
          size="sm"
          label={t("tempo.metronome.accent")}
          checked={settings.metronome_accent}
          onChange={(accent) => void sendEdit(transport, metronomeSettingsCommand({ accent }))}
        />
        <label className="eth-metronome__field">
          <span className="eth-metronome__label">{t("tempo.metronome.sound")}</span>
          <Select<MetronomeSound>
            size="sm"
            aria-label={t("tempo.metronome.sound_label")}
            options={METRONOME_SOUNDS.map((o) => ({ value: o.value, label: t(o.label) }))}
            value={settings.metronome_sound}
            onChange={(sound) => void sendEdit(transport, metronomeSettingsCommand({ sound }))}
          />
        </label>
        <p className="eth-metronome__hint">{t("tempo.metronome.hint")}</p>
      </div>
    </Popover>
  );
}
