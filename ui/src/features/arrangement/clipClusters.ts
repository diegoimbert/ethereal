/**
 * Level of detail for zoomed-out lanes: clips too small to show on their own are merged
 * into clusters, drawn as bars on one canvas per lane (see `ClusterLayer`) instead of one
 * element per clip.
 *
 * - A clip is "small" when it is narrower than `SMALL_CLIP_PX` at the zoom level.
 * - Small clips are swept in start order: a clip joins the current cluster when it starts
 *   less than `MERGE_GAP_PX` after the cluster's end, otherwise it starts a new cluster.
 *   Cluster edges stay exact (no pixel binning).
 * - The zoom level is the view's zoom rounded down to a power of two, so clusters only
 *   change when the zoom crosses a level (every 2×), never while panning or zooming within
 *   a level, and they can be memoized per lane and level.
 * - A cluster of one clip is just a clip again (drawn normally).
 */

import type { Beats } from "@/generated";
import type { LaneItem } from "./laneItems";

export const SMALL_CLIP_PX = 22;
export const MERGE_GAP_PX = 5;

/** The zoom level for `pxPerBeat`: the power of two at or below it. */
export function zoomLevel(pxPerBeat: number): number {
  return 2 ** Math.floor(Math.log2(Math.max(1e-6, pxPerBeat)));
}

export interface ClipCluster {
  start: Beats;
  end: Beats;
  /** In start order; at least two. */
  items: LaneItem[];
}

export interface ClusteredLane {
  /** Drawn as normal clips. */
  singles: LaneItem[];
  clusters: ClipCluster[];
}

/**
 * Split a lane's items (sorted by start, see `laneItems`) into clips drawn normally and
 * clusters, at zoom level `level` (px per beat). Copy-drag ghosts are clustered apart from
 * real clips (they are drawn translucent and aren't interactive).
 */
export function clusterLane(items: ReadonlyArray<LaneItem>, level: number): ClusteredLane {
  const small = SMALL_CLIP_PX / level;
  const gap = MERGE_GAP_PX / level;
  const singles: LaneItem[] = [];
  const clusters: ClipCluster[] = [];
  for (const ghost of [false, true]) {
    let cur: ClipCluster | null = null;
    const flush = () => {
      if (!cur) return;
      if (cur.items.length > 1) clusters.push(cur);
      else singles.push(cur.items[0]!);
      cur = null;
    };
    for (const it of items) {
      if (it.ghost !== ghost) continue;
      if (it.bounds.length >= small) {
        singles.push(it);
        continue;
      }
      const start = it.bounds.start;
      const end = start + it.bounds.length;
      if (cur && start - cur.end < gap) {
        cur.items.push(it);
        cur.end = Math.max(cur.end, end);
      } else {
        flush();
        cur = { start, end, items: [it] };
      }
    }
    flush();
  }
  return { singles, clusters };
}

/** The cluster under timeline beat `at`, with `slopBeats` of tolerance on each side. */
export function clusterAt(clusters: ReadonlyArray<ClipCluster>, at: Beats, slopBeats: Beats): ClipCluster | null {
  for (const c of clusters) {
    if (c.items[0]!.ghost) continue;
    if (at >= c.start - slopBeats && at < c.end + slopBeats) return c;
  }
  return null;
}
