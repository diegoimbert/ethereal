/**
 * Small clips: clips narrower than `SMALL_CLIP_PX` on screen are drawn without their name
 * and hit-tested with a few px of slop (`smallClipAt`), so very thin ones stay clickable
 * (gl/scene.ts, gl/hitTest.ts).
 */

import type { Beats } from "@/generated";
import type { LaneItem } from "./laneItems";

export const SMALL_CLIP_PX = 48;

/**
 * The small clip under timeline beat `at`: the one containing it, else the nearest one
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
