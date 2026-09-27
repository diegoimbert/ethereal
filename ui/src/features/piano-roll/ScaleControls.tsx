/**
 * Piano-roll scale popover: which scale the clip's track uses (the project's, its own, or
 * none), that scale's root and type, and the local view options (highlight, fold to the
 * scale's rows). The scale is document state; the view options are per-editor UI state.
 */

import type { MusicalScale, TrackScale } from "@/generated";
import { scaleLabel } from "@/domain/scales";
import { Button, Popover, Select, Toggle } from "@/kit";
import { ScaleSelect } from "@/features/scale/ScaleSelect";

type Mode = TrackScale["type"];

const MODE_OPTIONS: ReadonlyArray<{ value: Mode; label: string }> = [
  { value: "FollowProject", label: "Follow Project Scale" },
  { value: "Custom", label: "Custom Track Scale" },
  { value: "Chromatic", label: "Chromatic / None" },
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
  return (
    <Popover
      aria-label="Scale"
      trigger={(p) => (
        <Button
          size="sm"
          active={only}
          title="Scale (a visual guide; notes remain unrestricted)"
          data-testid="scale-trigger"
          {...p}
        >
          {scaleLabel(scale)}…
        </Button>
      )}
    >
      <div className="eth-scale-controls" data-testid="scale-popover">
        <label className="eth-scale-controls__field">
          <span className="eth-scale-controls__label">Track</span>
          <Select<Mode> size="sm" aria-label="Track scale mode" value={mode} options={MODE_OPTIONS} onChange={onMode} />
        </label>
        <div className="eth-scale-controls__field">
          <span
            className="eth-scale-controls__label"
            title={mode === "FollowProject" ? "Edits the project scale, for every track that follows it" : "Edits this track's scale"}
          >
            {mode === "FollowProject" ? "Project" : mode === "Custom" ? "Track" : "None"}
          </span>
          <ScaleSelect label="Active scale" value={scale} disabled={mode === "Chromatic"} onChange={onScale} />
        </div>
        <Toggle size="sm" label="Highlight" checked={highlight} onChange={onHighlight} />
        <span title="Hide out-of-scale rows without changing notes. Turn off to draw any pitch.">
          <Toggle size="sm" label="Scale notes only" checked={only} onChange={onOnly} />
        </span>
      </div>
    </Popover>
  );
}
