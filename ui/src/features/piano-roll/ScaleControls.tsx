import type { MusicalScale, TrackScale } from "@/generated";
import { ScaleSelect } from "@/features/scale/ScaleSelect";

export function ScaleControls({ scale, mode, onMode, onScale, highlight, onHighlight, only, onOnly }: {
  scale: MusicalScale;
  mode: TrackScale["type"];
  onMode: (mode: TrackScale["type"]) => void;
  onScale: (value: MusicalScale) => void;
  highlight: boolean;
  onHighlight: (value: boolean) => void;
  only: boolean;
  onOnly: (value: boolean) => void;
}) {
  return <div className="eth-scale-controls">
    <span>Scale:</span>
    <span title={mode === "FollowProject" ? "Edits the Project Scale for all following tracks" : "Edits this track's scale"}>
      <ScaleSelect label="Active scale" value={scale} disabled={mode === "Chromatic"} onChange={onScale} />
    </span>
    <span className="eth-scale-origin">· {mode === "FollowProject" ? "Project" : mode === "Custom" ? "Track" : "None"}</span>
    <select className="eth-scale-source" aria-label="Track scale mode" value={mode}
      onChange={(e) => onMode(e.target.value as TrackScale["type"])}>
      <option value="FollowProject">Follow Project Scale</option>
      <option value="Custom">Custom Track Scale</option>
      <option value="Chromatic">Chromatic / None</option>
    </select>
    <label><input type="checkbox" checked={highlight} onChange={(e) => onHighlight(e.target.checked)} />Highlight</label>
    <label title="Hide out-of-scale rows without changing notes. Turn off to draw any pitch.">
      <input type="checkbox" checked={only} onChange={(e) => onOnly(e.target.checked)} />Scale notes only
    </label>
  </div>;
}
