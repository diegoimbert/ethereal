// Arranger pointer and viewport mapping between SONG coordinates (what travels: beats, a
// track and a 0..1 fraction of its row) and this user's SCREEN (their own zoom, scroll, row
// heights and folds). Pure; see docs/COLLAB.md §8.3 and §8.4.
import type { ArrangerPointer, ArrangerViewport, BeatRange, TrackId } from "@/generated";

/** A track row on screen (px in one coordinate space; `top` may be off-screen). */
export interface RowBox {
  track: TrackId;
  top: number;
  height: number;
}

/** The horizontal mapping: lanes start at `left` px, where `scrollBeats` is. */
export interface Lanes {
  left: number;
  pxPerBeat: number;
  scrollBeats: number;
}

/** Visible area of the lanes (px, same space as the rows). */
export interface Box {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

const clamp01 = (v: number) => Math.min(1, Math.max(0, v));

export function beatsToX(beats: number, lanes: Lanes): number {
  return lanes.left + (beats - lanes.scrollBeats) * lanes.pxPerBeat;
}

/** The row containing `y` (tops ascending or not; rows don't overlap). */
function rowAt(rows: ReadonlyArray<RowBox>, y: number): RowBox | undefined {
  return rows.find((r) => y >= r.top && y < r.top + r.height);
}

/**
 * The free space below the last track: from its bottom (`top`) to the bottom of the view
 * (`bottom`). Each user has their own; pointers in it travel as a fraction of it.
 */
export interface FreeSpace {
  top: number;
  bottom: number;
}

/** Height a pointer's free-space fraction spans: the visible free space, at least `min`. */
const freeHeight = (f: FreeSpace, min: number) => Math.max(f.bottom - f.top, min);

/**
 * Smallest `y` a free-space pointer sends: `track: null, y: 0` means the ruler (and is what
 * older peers send), so a pointer right below the last track still sends a little more.
 */
export const FREE_SPACE_MIN_Y = 1e-3;

/**
 * The song position under a screen point. Left of the lanes (the header column) counts as
 * the left edge; outside every row `track` is null, with `y` 0 over the ruler and, below
 * the last track (`free`), the fraction (> 0) of the way down the free space.
 */
export function screenToSong(
  x: number,
  y: number,
  rows: ReadonlyArray<RowBox>,
  lanes: Lanes,
  free?: FreeSpace,
  minFree = 0,
): ArrangerPointer {
  const beats = Math.max(0, lanes.scrollBeats + Math.max(0, x - lanes.left) / lanes.pxPerBeat);
  const row = rowAt(rows, y);
  if (!row || row.height <= 0) {
    if (free && y >= free.top) {
      const frac = (y - free.top) / freeHeight(free, minFree);
      return { beats, track: null, y: Math.min(1, Math.max(FREE_SPACE_MIN_Y, frac)) };
    }
    return { beats, track: null, y: 0 };
  }
  return { beats, track: row.track, y: clamp01((y - row.top) / row.height) };
}

/** Where a track shows locally: its own row, or its nearest visible ancestor (a folded group). */
export function visibleRowOf(
  track: TrackId,
  rows: ReadonlyArray<RowBox>,
  parentOf: (track: TrackId) => TrackId | null | undefined,
): { row: RowBox; folded: boolean } | null {
  const byId = new Map(rows.map((r) => [r.track, r]));
  let id: TrackId | null | undefined = track;
  for (let guard = 0; id != null && guard < 64; guard++) {
    const row = byId.get(id);
    if (row) return { row, folded: id !== track };
    id = parentOf(id);
  }
  return null;
}

export interface ScreenPoint {
  x: number;
  y: number;
  /** Shown on a folded group's row instead of its own (hidden) row. */
  folded: boolean;
}

/**
 * A peer's song pointer on this screen. `noTrackY`: where pointers over the ruler go.
 * Pointers below the last track (`track: null`, `y > 0`) land at the same fraction of this
 * user's free space (`free`, spanning at least `minFree`). A pointer inside a folded group
 * sits in the middle of the group's row. `null`: the track is unknown here (deleted), so
 * hide it.
 */
export function songToScreen(
  p: ArrangerPointer,
  rows: ReadonlyArray<RowBox>,
  lanes: Lanes,
  parentOf: (track: TrackId) => TrackId | null | undefined,
  noTrackY: number,
  free?: FreeSpace,
  minFree = 0,
): ScreenPoint | null {
  const x = beatsToX(p.beats, lanes);
  if (p.track === null) {
    if (free && p.y > 0) return { x, y: free.top + clamp01(p.y) * freeHeight(free, minFree), folded: false };
    return { x, y: noTrackY, folded: false };
  }
  const hit = visibleRowOf(p.track, rows, parentOf);
  if (!hit) return null;
  const frac = hit.folded ? 0.5 : clamp01(p.y);
  return { x, y: hit.row.top + frac * hit.row.height, folded: hit.folded };
}

export type Edge = "left" | "right" | "top" | "bottom";

/** Clamp a point into the visible box; `edge` says which side it was beyond (null: inside). */
export function clampToBox(x: number, y: number, box: Box): { x: number; y: number; edge: Edge | null } {
  const cx = Math.min(box.x1, Math.max(box.x0, x));
  const cy = Math.min(box.y1, Math.max(box.y0, y));
  let edge: Edge | null = null;
  if (x < box.x0) edge = "left";
  else if (x > box.x1) edge = "right";
  else if (y < box.y0) edge = "top";
  else if (y > box.y1) edge = "bottom";
  return { x: cx, y: cy, edge };
}

/**
 * This user's viewport: the visible beat range, and the row at the top edge of the scrolled
 * rows (`rows` in content px, i.e. `top` from the first row; `scrollTop` the scroll offset).
 */
export function viewportOf(rows: ReadonlyArray<RowBox>, scrollTop: number, visible: BeatRange): ArrangerViewport {
  const row = rowAt(rows, scrollTop) ?? (scrollTop < 0 ? rows[0] : undefined);
  return {
    start: visible.start,
    end: visible.end,
    top_track: row?.track ?? null,
    top_offset: row && row.height > 0 ? clamp01((scrollTop - row.top) / row.height) : 0,
  };
}

/** The scroll offset putting a leader's `top_track` (or its folded group) at the top; null: unknown track. */
export function scrollTopFor(
  v: ArrangerViewport,
  rows: ReadonlyArray<RowBox>,
  parentOf: (track: TrackId) => TrackId | null | undefined,
): number | null {
  if (v.top_track === null) return 0;
  const hit = visibleRowOf(v.top_track, rows, parentOf);
  if (!hit) return null;
  return hit.row.top + (hit.folded ? 0 : clamp01(v.top_offset) * hit.row.height);
}
