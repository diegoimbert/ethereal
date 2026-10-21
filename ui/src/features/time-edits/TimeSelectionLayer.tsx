import "./time-edits.css";
import type { TrackId } from "@/generated";
import { useViewport, type TimelineViewStore } from "@/timeline";
import { isMarker, useTimeSelection } from "./store";

interface LayerRow {
  track: { id: TrackId };
  y: number;
  height: number;
}

/**
 * The time selection drawn over the lanes of its tracks (content px, lanes start at
 * `headerWidth`), with its edges; at zero length, the insert marker (a thin blinking line),
 * across the whole row (its automation lanes too). A range in one automation lane is drawn
 * by the lane (`AutomationLaneView`).
 * Pointer events pass through.
 */
export function TimeSelectionLayer({ rows, view, headerWidth }: { rows: ReadonlyArray<LayerRow>; view: TimelineViewStore; headerWidth: number }) {
  const sel = useTimeSelection((s) => s.selection);
  const vp = useViewport(view);
  if (!sel) return null;
  const left = headerWidth + (sel.start - vp.scrollBeats) * vp.pxPerBeat;
  const width = (sel.end - sel.start) * vp.pxPerBeat;
  const shown = rows.filter((r) => sel.tracks.includes(r.track.id));
  // One automation lane's range is drawn by that lane (it covers only the lane).
  const laneRange = sel.automationLane !== undefined;
  if (isMarker(sel)) {
    if (left < headerWidth) return null;
    return (
      <>
        {shown.map((r) => (
          <div
            key={r.track.id}
            className="eth-insert-marker"
            data-testid="insert-marker"
            data-track={r.track.id}
            data-beats={sel.start}
            style={{ left, top: r.y, height: r.height }}
          />
        ))}
      </>
    );
  }
  if (laneRange) return null;
  return (
    <>
      {shown.map((r) => (
        <div
          key={r.track.id}
          className="eth-time-sel"
          data-testid="time-selection"
          data-track={r.track.id}
          style={{
            left: Math.max(left, headerWidth),
            top: r.y,
            width: Math.max(0, left + width - Math.max(left, headerWidth)),
            height: r.height,
          }}
        />
      ))}
    </>
  );
}
