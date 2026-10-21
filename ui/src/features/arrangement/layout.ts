/**
 * Vertical layout of the arrangement (pure): which tracks are shown (folded groups hide
 * their descendants), their order, depth and y offsets. Clip hit testing (marquee) and
 * drag-between-tracks use this model instead of measuring the DOM.
 *
 * Each row is its lane height (`TRACK_HEIGHT` unless resized, see `laneHeight`) plus the
 * height of its automation slot (0 until ui-automation mounts lanes there; pass their
 * heights via `automationHeight`).
 *
 * v0.2 (`comping`): expanded take lanes sit right under the main lane and count as part of
 * the row's lane height (`laneHeight` = main lane + `takesHeight`), so the automation slot
 * stays `height - laneHeight`. Clips live in the main lane only (`mainHeight`).
 */

import type { Clip, Track, TrackId } from "@/generated";
import type { Rect } from "@/timeline";
import { beatsToPx, type TimelineViewport } from "@/timeline";
import { isArrangementClip, startOf } from "./clipTime";

/** Default width of the track header column (resizable: `useArrangementUi().headerWidth`). */
export const HEADER_WIDTH = 232;
export const MIN_HEADER_WIDTH = 180;
/** A phone's header column (fixed, not resizable there): narrower than the desktop's minimum. */
export const PHONE_HEADER_WIDTH = 140;
/** Under this width the header is narrow: compact, its buttons are smaller (they cover the name). */
export const NARROW_HEADER_WIDTH = 220;
export const MAX_HEADER_WIDTH = 400;

/** How far each group level shifts its tracks' headers right (`--eth-size-track-indent`). */
export const HEADER_INDENT_PX = 12;

export function clampHeaderWidth(px: number): number {
  return Math.min(MAX_HEADER_WIDTH, Math.max(MIN_HEADER_WIDTH, px));
}

/** The deepest group nesting among `tracks` (0: no track inside a group). */
export function maxTrackDepth(tracks: ReadonlyArray<Track>): number {
  const byId = new Map(tracks.map((t) => [t.id, t]));
  let max = 0;
  for (const t of tracks) max = Math.max(max, depthOf(t, byId));
  return max;
}
/** Default lane height. Resized lanes stay within `[MIN_TRACK_HEIGHT, MAX_TRACK_HEIGHT]`. */
export const TRACK_HEIGHT = 64;
export const MIN_TRACK_HEIGHT = 24;
export const MAX_TRACK_HEIGHT = 320;
/** Dragging a lane's edge resizes it in these increments. */
export const TRACK_HEIGHT_STEP = 8;

export function clampTrackHeight(h: number): number {
  return Math.min(MAX_TRACK_HEIGHT, Math.max(MIN_TRACK_HEIGHT, h));
}
/** Height of the empty drop area below the last track. */
export const DROP_AREA_HEIGHT = 80;

export interface Row {
  track: Track;
  depth: number;
  /** Top of the row in content px (0 = first row). */
  y: number;
  /** Lane height: the main lane (clips live here) plus the take lanes (`takesHeight`). */
  laneHeight: number;
  /** v0.2 (`comping`): height of the expanded take lanes under the main lane (0 = none). */
  takesHeight?: number;
  /** Total height including the automation slot. */
  height: number;
}

/**
 * Tracks the arrangement shows, from all tracks in display order (`tracksOrdered`):
 * descendants of folded groups are hidden, returns and master go to the bottom.
 */
export function arrangementTracks(ordered: ReadonlyArray<Track>, folded: ReadonlySet<TrackId>): Track[] {
  const byId = new Map(ordered.map((t) => [t.id, t]));
  const hidden = (t: Track): boolean => {
    let p = t.parent;
    let guard = 0;
    while (p !== null && guard++ < 64) {
      if (folded.has(p)) return true;
      p = byId.get(p)?.parent ?? null;
    }
    return false;
  };
  const rank = (t: Track) => (t.kind === "Master" ? 2 : t.kind === "Return" ? 1 : 0);
  return ordered
    .filter((t) => !hidden(t))
    .map((t, i) => ({ t, i }))
    .sort((a, b) => rank(a.t) - rank(b.t) || a.i - b.i)
    .map((x) => x.t);
}

function depthOf(t: Track, byId: ReadonlyMap<TrackId, Track>): number {
  let depth = 0;
  let p = t.parent;
  while (p !== null && depth < 64) {
    depth++;
    p = byId.get(p)?.parent ?? null;
  }
  return depth;
}

export function layoutRows(
  ordered: ReadonlyArray<Track>,
  folded: ReadonlySet<TrackId>,
  automationHeight: (track: TrackId) => number = () => 0,
  laneHeightOf: (track: TrackId) => number = () => TRACK_HEIGHT,
  takesHeightOf: (track: TrackId) => number = () => 0,
): Row[] {
  const byId = new Map(ordered.map((t) => [t.id, t]));
  const tracks = arrangementTracks(ordered, folded);
  let y = 0;
  const rows: Row[] = [];
  tracks.forEach((track) => {
    const takesHeight = Math.round(takesHeightOf(track.id));
    const laneHeight = Math.round(laneHeightOf(track.id)) + takesHeight;
    const height = laneHeight + automationHeight(track.id);
    rows.push({ track, depth: depthOf(track, byId), y, laneHeight, height, ...(takesHeight > 0 ? { takesHeight } : {}) });
    y += height;
  });
  return rows;
}

/** Height of a row's main lane (where its clips are), without the take lanes. */
export function mainHeight(row: Row): number {
  return row.laneHeight - (row.takesHeight ?? 0);
}

/** Total height of the rows. */
export function rowsHeight(rows: ReadonlyArray<Row>): number {
  const last = rows[rows.length - 1];
  return last ? last.y + last.height : 0;
}

/** Index of the row at content y (`-1` above, `rows.length` below the last one). */
export function rowIndexAt(rows: ReadonlyArray<Row>, y: number): number {
  if (y < 0) return -1;
  for (let i = 0; i < rows.length; i++) if (y < rows[i]!.y + rows[i]!.height) return i;
  return rows.length;
}

/** Whether a track can hold clips of this content. */
export function acceptsClip(track: Track, content: Clip["content"]["type"]): boolean {
  return track.kind === (content === "Midi" ? "Midi" : "Audio");
}

/**
 * Clip rectangles in content px (x includes the header column), for marquee hit tests.
 * `clips` may override position/track (drag preview).
 */
export function clipRects(
  rows: ReadonlyArray<Row>,
  clips: Iterable<Clip>,
  vp: TimelineViewport,
  headerWidth = HEADER_WIDTH,
): Array<{ id: Clip["id"]; rect: Rect }> {
  const rowOf = new Map(rows.map((r) => [r.track.id, r]));
  const out: Array<{ id: Clip["id"]; rect: Rect }> = [];
  for (const c of clips) {
    const row = rowOf.get(c.track);
    if (!row || !isArrangementClip(c)) continue;
    const x0 = headerWidth + beatsToPx(startOf(c), vp);
    out.push({ id: c.id, rect: { x0, x1: x0 + c.length * vp.pxPerBeat, y0: row.y, y1: row.y + mainHeight(row) } });
  }
  return out;
}
