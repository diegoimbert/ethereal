/**
 * Timeline primitives shared by arrangement, piano-roll and automation.
 *
 * OWNERSHIP: the `ui-timeline` node owns `ui/src/timeline/**`. Only edit files inside this folder.
 *
 * Intended scope (to be built by `ui-timeline`):
 * - time ↔ pixel mapping in bars/beats/seconds (seconds via the tempo map),
 * - a zoom/scroll store (per view) with zoom-around-cursor,
 * - grid resolution (adaptive to zoom) + snapping,
 * - a selection model (time range + item ids),
 * - a `Ruler` component (bars/beats labels, loop brace, playhead).
 *
 * This foundation stub only fixes the tiny core API below; extend it, keep it stable.
 * Time positions are in beats (quarter notes), matching the engine's musical time.
 */

/** Horizontal viewport of a timeline. */
export interface TimelineViewport {
  /** Zoom: pixels per beat. */
  pxPerBeat: number;
  /** Beat at the left edge of the viewport. */
  scrollBeats: number;
}

/** Beat position → x pixel within the viewport. */
export function beatsToPx(beats: number, vp: TimelineViewport): number {
  return (beats - vp.scrollBeats) * vp.pxPerBeat;
}

/** x pixel within the viewport → beat position. */
export function pxToBeats(px: number, vp: TimelineViewport): number {
  return px / vp.pxPerBeat + vp.scrollBeats;
}
