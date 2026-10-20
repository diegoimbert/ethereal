/**
 * The time range's two handles on touch screens (the design's §5 "Range"): like iOS text
 * selection, a grip on each end that drags that end. Pure: the range, the rows, the view.
 *
 * A handle's target is `TOUCH_TARGET_PX` wide, centred on its edge, over the range's rows (from
 * the first selected row's top to the last one's bottom). On a range narrower than the target,
 * the nearer edge wins.
 */
import type { Beats, TrackId } from "@/generated";
import { TOUCH_TARGET_PX } from "./clipTouch";

export type RangeEdge = "start" | "end";

interface EdgeRow {
  track: { id: TrackId };
  y: number;
  height: number;
}

/** The range's box in lanes px (x from the lanes' left, y in content px), or null if none of its rows is shown. */
export function rangeBox(
  range: { start: Beats; end: Beats; tracks: ReadonlyArray<TrackId> },
  rows: ReadonlyArray<EdgeRow>,
  vp: { pxPerBeat: number; scrollBeats: Beats },
): { x0: number; x1: number; y0: number; y1: number } | null {
  const shown = rows.filter((r) => range.tracks.includes(r.track.id));
  if (shown.length === 0) return null;
  return {
    x0: (range.start - vp.scrollBeats) * vp.pxPerBeat,
    x1: (range.end - vp.scrollBeats) * vp.pxPerBeat,
    y0: Math.min(...shown.map((r) => r.y)),
    y1: Math.max(...shown.map((r) => r.y + r.height)),
  };
}

/** The handle a finger at (`x`, `y`) (lanes px) grabs, if any. */
export function rangeHandleAt(box: { x0: number; x1: number; y0: number; y1: number }, x: number, y: number): RangeEdge | null {
  if (y < box.y0 || y >= box.y1) return null;
  const half = TOUCH_TARGET_PX / 2;
  const ds = Math.abs(x - box.x0);
  const de = Math.abs(x - box.x1);
  if (ds > half && de > half) return null;
  return ds < de ? "start" : "end";
}

/** The range after dragging `edge` to `beats` (an end dragged past the other swaps them). */
export function dragRangeEdge(range: { start: Beats; end: Beats }, edge: RangeEdge, beats: Beats): { start: Beats; end: Beats } {
  const fixed = edge === "start" ? range.end : range.start;
  const moved = Math.max(0, beats);
  return { start: Math.min(fixed, moved), end: Math.max(fixed, moved) };
}
