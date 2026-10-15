/**
 * Shift-click on a time selection (owner request, every view with one: arrangement clip
 * lanes, automation lanes, the piano roll): a click OUTSIDE the range extends it so the
 * click becomes the edge on that side; a click INSIDE retracts it, the edge nearer to the
 * click moving to it (exactly in the middle: the end moves). A zero-length range (the insert
 * marker) extends from the marker. The caller snaps the click (grid; Alt: free), or passes
 * `snap`.
 */

export interface BeatRange {
  start: number;
  end: number;
}

/** Which edge a Shift-click at `at` moves (the other one is the anchor a Shift-drag pivots on). */
export function movedEdge(range: BeatRange, at: number): "start" | "end" {
  if (at < range.start) return "start";
  if (at > range.end) return "end";
  return at - range.start < range.end - at ? "start" : "end";
}

/** `range` after a Shift-click at `clickBeat` (snapped by `snap` if given), start <= end. */
export function extendOrRetract<R extends BeatRange>(range: R, clickBeat: number, snap?: (beats: number) => number): R {
  const at = Math.max(0, snap ? snap(clickBeat) : clickBeat);
  return movedEdge(range, at) === "start" ? { ...range, start: at } : { ...range, end: at };
}
