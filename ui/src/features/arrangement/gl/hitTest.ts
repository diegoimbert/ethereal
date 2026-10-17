/**
 * Hit-testing of the WebGL arranger's clips (pure), with the arranger's part
 * priorities (its DOM stacking):
 *
 * 1. clips drawn above the others first: selected and dragged clips (`z-index: 2`), then
 *    the rest, later starts above earlier ones;
 * 2. within a clip: its edge handles (`handleWidth`: 25% of the clip, clamped to 3–6 px,
 *    full height, above the title), then its title band (the grab zone);
 * 3. a clip's body lets the pointer through (to clips below, then to the lane: locate,
 *    marquee); it is reported as `body` only when nothing interactive is hit;
 * 4. clips narrower than `SMALL_CLIP_PX` (their title band only)
 *    are hit anywhere in the lane's height, within `SMALL_SLOP_PX`, like its title bar, and
 *    only when no clip element was hit. They keep their edge handles (same widths, over the
 *    lane's height) down to `MIN_RESIZE_PX`; narrower ones only move (zoom in to trim them).
 *
 * Ghosts (copy-drag previews) are never hit.
 */

import type { ClipId, TrackId } from "@/generated";
import { CLIP_INSET, CLIP_MIN_PX, CLIP_TITLE } from "../laneBodyGeometry";
import type { LaneItem } from "../laneItems";
import { SMALL_CLIP_PX, smallClipAt } from "../smallClips";

/** Slop around painted small clips (px), as `TrackRow`'s `SMALL_SLOP_PX`. */
export const SMALL_SLOP_PX = 3;
/** Widest edge handle (`--eth-size-edge-handle`). */
export const EDGE_HANDLE_PX = 6;
/** Narrowest edge handle (on clips under 24 px, where 25% would be thinner). */
export const EDGE_HANDLE_MIN_PX = 3;
/**
 * Clips narrower than this on screen have no edge handles (they only move): two 3 px
 * handles leave a 4 px grab zone at 10 px. Ableton / Bitwig likewise trim clips of a few px.
 */
export const MIN_RESIZE_PX = 10;

/** `layer`: a phase-2 layer's own part (`Hit.layer`, see layers.ts). */
export type HitPart = "header" | "resize-start" | "resize-end" | "body" | "small" | "layer";

export interface Hit {
  clip: ClipId;
  track: TrackId;
  part: HitPart;
  /** The layer that took the pointer (`part === "layer"`). */
  layer?: string;
  /** Layer-specific details of the hit (e.g. which fade handle). */
  data?: unknown;
}

export interface LaneView {
  scrollBeats: number;
  pxPerBeat: number;
}

/** A clip box's x extent in lane px (at least `CLIP_MIN_PX` wide). */
export function boxX(it: LaneItem, v: LaneView): [number, number] {
  const x0 = (it.bounds.start - v.scrollBeats) * v.pxPerBeat;
  return [x0, Math.max(x0 + CLIP_MIN_PX, x0 + it.bounds.length * v.pxPerBeat)];
}

/**
 * Width of a clip box's edge handles: a quarter of the box, clamped to
 * `[EDGE_HANDLE_MIN_PX, EDGE_HANDLE_PX]` (0 under `MIN_RESIZE_PX`: no handles). Mirrored by
 * the hover highlight (shaders.ts, canvas2d.ts).
 */
export function handleWidth(x0: number, x1: number): number {
  const w = x1 - x0;
  if (w < MIN_RESIZE_PX) return 0;
  return Math.max(EDGE_HANDLE_MIN_PX, Math.min(EDGE_HANDLE_PX, 0.25 * w));
}

/** The edge handle of the box `[x0, x1)` at `x` (inside the box), if any. */
export function edgeAt(x: number, x0: number, x1: number): "resize-start" | "resize-end" | null {
  const hw = handleWidth(x0, x1);
  if (hw === 0 || x < x0 || x >= x1) return null;
  if (x < x0 + hw) return "resize-start";
  if (x >= x1 - hw) return "resize-end";
  return null;
}

/** Whether a clip is drawn as a small clip (no body; handles from `MIN_RESIZE_PX`) at this zoom. */
export function isSmall(it: LaneItem, v: LaneView): boolean {
  return it.bounds.length * v.pxPerBeat < SMALL_CLIP_PX;
}

/** Whether an item is raised above the lane's other clips (selected or dragged). */
export function isRaised(it: LaneItem, selected: ReadonlySet<ClipId>): boolean {
  return it.dragging || (!it.ghost && selected.has(it.clip.id));
}

/**
 * The clip part at lane px `x` and `y` (from the lane's top) in a lane `mainHeight` tall,
 * among `items` (sorted by start, as `laneItems`).
 */
export function hitLane(
  items: ReadonlyArray<LaneItem>,
  selected: ReadonlySet<ClipId>,
  track: TrackId,
  x: number,
  y: number,
  mainHeight: number,
  v: LaneView,
): Hit | null {
  if (y < 0 || y >= mainHeight) return null;
  const top = CLIP_INSET;
  const bottom = mainHeight - CLIP_INSET;
  let body: Hit | null = null;
  const order: LaneItem[] = [];
  for (let i = items.length - 1; i >= 0; i--) if (isRaised(items[i]!, selected)) order.push(items[i]!);
  for (let i = items.length - 1; i >= 0; i--) if (!isRaised(items[i]!, selected)) order.push(items[i]!);
  const small: LaneItem[] = [];
  for (const it of order) {
    if (it.ghost) continue;
    if (isSmall(it, v)) {
      small.push(it);
      continue;
    }
    const [x0, x1] = boxX(it, v);
    if (x < x0 || x >= x1 || y < top || y >= bottom) continue;
    const at = (part: HitPart): Hit => ({ clip: it.clip.id, track, part });
    const edge = edgeAt(x, x0, x1);
    if (edge) return at(edge);
    if (y < top + CLIP_TITLE) return at("header");
    body ??= at("body");
  }
  if (small.length) {
    small.sort((a, b) => a.bounds.start - b.bounds.start);
    const beat = x / v.pxPerBeat + v.scrollBeats;
    const it = smallClipAt(small, beat, SMALL_SLOP_PX / v.pxPerBeat);
    if (it) {
      const [x0, x1] = boxX(it, v);
      return { clip: it.clip.id, track, part: edgeAt(x, x0, x1) ?? "small" };
    }
  }
  return body;
}

/** The cursor over a part. */
export function cursorFor(part: HitPart | null): string {
  if (part === "header") return "grab";
  if (part === "resize-start" || part === "resize-end") return "ew-resize";
  return "";
}
