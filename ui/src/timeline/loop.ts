/** Loop-region editing math (pure), used by `Ruler`. */

import type { BeatRange, Beats } from "@/generated";

export type LoopHandle = "start" | "end" | "move";

/** Minimum loop length in beats. */
export const MIN_LOOP_BEATS = 1 / 16;

/**
 * New loop region after dragging `handle` by `delta` beats from `region` (pure).
 * `snap` maps a position to the grid; moves snap the start and keep the length.
 */
export function applyLoopDrag(
  region: BeatRange,
  handle: LoopHandle,
  delta: Beats,
  snap: (b: Beats) => Beats = (b) => b,
): BeatRange {
  const len = region.end - region.start;
  switch (handle) {
    case "move": {
      const start = Math.max(0, snap(region.start + delta));
      return { start, end: start + len };
    }
    case "start": {
      const start = Math.max(0, Math.min(snap(region.start + delta), region.end - MIN_LOOP_BEATS));
      return { start, end: region.end };
    }
    case "end": {
      const end = Math.max(snap(region.end + delta), region.start + MIN_LOOP_BEATS);
      return { start: region.start, end };
    }
  }
}

/** A loop region spanning two positions (for shift-drag drawing). */
export function loopFromPoints(a: Beats, b: Beats): BeatRange {
  const start = Math.max(0, Math.min(a, b));
  return { start, end: Math.max(start + MIN_LOOP_BEATS, Math.max(a, b)) };
}
