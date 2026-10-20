/**
 * Which clip a finger grabs, and how (the design's §4 "Trim handles"). Pure: clip boxes
 * in content px (`clipRects`), the selection, a point.
 *
 * Only a selected clip has trim handles on touch, and they are wide: a `TOUCH_TARGET_PX` target
 * centred on each edge, half outside the clip and half inside (the inside part capped at a third
 * of the clip's width, so a narrow clip keeps a body to grab). A clip under `MIN_TRIM_PX` wide on
 * screen has none (zoom in, or use the numbers). Elsewhere on a clip is its body (a move).
 */
import type { Rect } from "@/timeline";

export const TOUCH_TARGET_PX = 44;
export const MIN_TRIM_PX = 24;

export type ClipGrab = "move" | "resize-start" | "resize-end";

export interface ClipBox {
  id: string;
  rect: Rect;
}

/** The trim zones of a clip box (content px), or null when it is too narrow for handles. */
export function trimZones(r: Rect): { start: [number, number]; end: [number, number] } | null {
  const w = r.x1 - r.x0;
  if (w < MIN_TRIM_PX) return null;
  const half = TOUCH_TARGET_PX / 2;
  const inside = Math.min(half, w / 3);
  return { start: [r.x0 - half, r.x0 + inside], end: [r.x1 - inside, r.x1 + half] };
}

export function clipGrabAt(boxes: ReadonlyArray<ClipBox>, selected: ReadonlySet<string>, x: number, y: number): { clip: string; grab: ClipGrab } | null {
  // Trim handles of selected clips first (they reach outside the clip), nearest edge wins.
  let best: { clip: string; grab: ClipGrab; d: number } | null = null;
  for (const { id, rect } of boxes) {
    if (!selected.has(id) || y < rect.y0 || y >= rect.y1) continue;
    const z = trimZones(rect);
    if (!z) continue;
    if (x >= z.start[0] && x <= z.start[1]) {
      const d = Math.abs(x - rect.x0);
      if (!best || d < best.d) best = { clip: id, grab: "resize-start", d };
    }
    if (x >= z.end[0] && x <= z.end[1]) {
      const d = Math.abs(x - rect.x1);
      if (!best || d < best.d) best = { clip: id, grab: "resize-end", d };
    }
  }
  if (best) return { clip: best.clip, grab: best.grab };
  // A body: the topmost (last drawn) clip under the finger.
  for (let i = boxes.length - 1; i >= 0; i--) {
    const { id, rect } = boxes[i]!;
    if (x >= rect.x0 && x < rect.x1 && y >= rect.y0 && y < rect.y1) return { clip: id, grab: "move" };
  }
  return null;
}

/** Edge auto-scroll speed (px/s, signed) for a finger at `p` within `[lo, hi]`: 0 outside the zones. */
export const EDGE_ZONE_PX = 48;
export const EDGE_MAX_SPEED = 900;

export function edgeSpeed(p: number, lo: number, hi: number): number {
  if (p < lo + EDGE_ZONE_PX) return -EDGE_MAX_SPEED * Math.min(1, (lo + EDGE_ZONE_PX - p) / EDGE_ZONE_PX);
  if (p > hi - EDGE_ZONE_PX) return EDGE_MAX_SPEED * Math.min(1, (p - (hi - EDGE_ZONE_PX)) / EDGE_ZONE_PX);
  return 0;
}
