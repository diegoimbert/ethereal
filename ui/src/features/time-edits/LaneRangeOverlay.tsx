import { useViewport, type TimelineViewStore } from "@/timeline";
import { rangeOf, useTimeSelection } from "./store";

/**
 * A lane's own time section (a time selection drawn in one automation lane or the tempo
 * lane: `automationLane` is the lane's key), drawn like the clip lanes' (`TimeSelectionLayer`),
 * over that lane only. Its own component: it follows the view (pan / zoom) without
 * re-rendering the lane.
 */
export function LaneRangeOverlay({ view, track, targetKey, height }: { view: TimelineViewStore; track: string; targetKey: string; height: number }) {
  const sel = useTimeSelection((s) => rangeOf(s.selection));
  const vp = useViewport(view);
  if (!sel || sel.automationLane !== targetKey || sel.tracks[0] !== track) return null;
  const left = (sel.start - vp.scrollBeats) * vp.pxPerBeat;
  const right = (sel.end - vp.scrollBeats) * vp.pxPerBeat;
  if (right <= 0) return null;
  return (
    <div
      className="eth-time-sel eth-auto-lane__range"
      data-testid="automation-range"
      data-start={sel.start}
      data-end={sel.end}
      style={{ left: Math.max(0, left), top: 0, width: right - Math.max(0, left), height }}
    />
  );
}
