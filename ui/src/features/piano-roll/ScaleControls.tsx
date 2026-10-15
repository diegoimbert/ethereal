/**
 * Piano-roll scale popover: which scale the clip's track uses (the project's, its own, or
 * none), that scale's root and type, and the local view options (highlight, fold to the
 * scale's rows). The scale is document state; the view options are per-editor UI state.
 */

import type { MusicalScale, TrackScale } from "@/generated";
import { scaleLabel } from "@/domain/scales";
import { Button, Popover, Select, Toggle } from "@/kit";
import { ScaleSelect } from "@/features/scale/ScaleSelect";
import { useT, type MessageKey } from "@/i18n";

type Mode = TrackScale["type"];

const MODE_OPTIONS: ReadonlyArray<{ value: Mode; label: MessageKey }> = [
  { value: "FollowProject", label: "piano_roll.scale.mode_follow" },
  { value: "Custom", label: "piano_roll.scale.mode_custom" },
  { value: "Chromatic", label: "scale.kind.chromatic" },
];

export interface ScaleControlsProps {
  scale: MusicalScale;
  mode: Mode;
  onMode: (mode: Mode) => void;
  onScale: (value: MusicalScale) => void;
  highlight: boolean;
  onHighlight: (value: boolean) => void;
  only: boolean;
  onOnly: (value: boolean) => void;
}

export function ScaleControls({ scale, mode, onMode, onScale, highlight, onHighlight, only, onOnly }: ScaleControlsProps) {
  const t = useT();
  const modeOptions = MODE_OPTIONS.map((o) => ({ value: o.value, label: t(o.label) }));
  return (
    <Popover
      aria-label={t("piano_roll.scale.popover")}
      trigger={(p) => (
        <Button size="sm" active={only} title={t("piano_roll.scale.trigger_title")} data-testid="scale-trigger" {...p}>
          {scaleLabel(scale)}…
        </Button>
      )}
    >
      <div className="eth-scale-controls" data-testid="scale-popover">
        <label className="eth-scale-controls__field">
          <span className="eth-scale-controls__label">{t("piano_roll.scale.track")}</span>
          <Select<Mode> size="sm" aria-label={t("piano_roll.scale.track_mode")} value={mode} options={modeOptions} onChange={onMode} />
        </label>
        <div className="eth-scale-controls__field">
          <span
            className="eth-scale-controls__label"
            title={mode === "FollowProject" ? t("piano_roll.scale.edits_project") : t("piano_roll.scale.edits_track")}
          >
            {mode === "FollowProject" ? t("piano_roll.scale.project") : mode === "Custom" ? t("piano_roll.scale.track") : t("piano_roll.scale.none")}
          </span>
          <ScaleSelect label={t("piano_roll.scale.active")} value={scale} disabled={mode === "Chromatic"} onChange={onScale} />
        </div>
        <Toggle size="sm" label={t("piano_roll.scale.highlight")} checked={highlight} onChange={onHighlight} />
        <span title={t("piano_roll.scale.only_title")}>
          <Toggle size="sm" label={t("piano_roll.scale.only")} checked={only} onChange={onOnly} />
        </span>
      </div>
    </Popover>
  );
}
