/**
 * Level of detail for zoomed-out lanes, purely an optimization: clips narrower than
 * `SMALL_CLIP_PX` on screen aren't DOM elements. `SmallClipsLayer` paints them, looking
 * like clips, on one canvas per lane, and the lane hit-tests them so they behave exactly
 * like clips (click, drag, copy, double-click, context menu). Wider clips are normal
 * `ClipView`s, so nothing changes when zoomed in.
 */

import type { Beats } from "@/generated";
import type { LaneItem } from "./laneItems";

export const SMALL_CLIP_PX = 48;

export interface SplitLane {
  /** Drawn as normal clips. */
  singles: LaneItem[];
  /** Painted on the lane canvas (in start order). */
  small: LaneItem[];
}

/** Split a lane's items (sorted by start) at zoom `pxPerBeat`. */
export function splitSmallClips(items: ReadonlyArray<LaneItem>, pxPerBeat: number): SplitLane {
  const small = SMALL_CLIP_PX / pxPerBeat;
  const out: SplitLane = { singles: [], small: [] };
  for (const it of items) (it.bounds.length < small ? out.small : out.singles).push(it);
  return out;
}

/**
 * The painted clip under timeline beat `at`: the one containing it, else the nearest one
 * within `slopBeats` (very thin clips stay clickable). Ghosts aren't interactive.
 */
export function smallClipAt(small: ReadonlyArray<LaneItem>, at: Beats, slopBeats: Beats): LaneItem | null {
  let best: LaneItem | null = null;
  let bestDist = slopBeats;
  for (const it of small) {
    if (it.ghost) continue;
    const start = it.bounds.start;
    const end = start + it.bounds.length;
    const d = at < start ? start - at : at >= end ? at - end : 0;
    if (d === 0) return it;
    if (d <= bestDist) {
      best = it;
      bestDist = d;
    }
  }
  return best;
}
