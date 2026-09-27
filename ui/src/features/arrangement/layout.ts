/**
 * Vertical layout of the arrangement (pure): which tracks are shown (folded groups hide
 * their descendants), their order, depth and y offsets. Clip hit testing (marquee) and
 * drag-between-tracks use this model instead of measuring the DOM.
 *
 * Each row is its lane height (`TRACK_HEIGHT` unless resized, see `laneHeight`) plus the
 * height of its automation slot (0 until ui-automation mounts lanes there; pass their
 * heights via `automationHeight`).
 */

import type { Clip, Track, TrackId } from "@/generated";
import type { Rect } from "@/timeline";
import { beatsToPx, type TimelineViewport } from "@/timeline";
import { isArrangementClip, startOf } from "./clipTime";

/** Default width of the track header column (resizable: `useArrangementUi().headerWidth`). */
export const HEADER_WIDTH = 200;
export const MIN_HEADER_WIDTH = 140;
export const MAX_HEADER_WIDTH = 400;

export function clampHeaderWidth(px: number): number {
  return Math.min(MAX_HEADER_WIDTH, Math.max(MIN_HEADER_WIDTH, px));
}
/** Default lane height. Resized lanes stay within `[MIN_TRACK_HEIGHT, MAX_TRACK_HEIGHT]`. */
export const TRACK_HEIGHT = 56;
export const MIN_TRACK_HEIGHT = 24;
export const MAX_TRACK_HEIGHT = 320;
/** Dragging a lane's edge resizes it in these increments. */
export const TRACK_HEIGHT_STEP = 8;

export function clampTrackHeight(h: number): number {
  return Math.min(MAX_TRACK_HEIGHT, Math.max(MIN_TRACK_HEIGHT, h));
}
/** Height of the empty drop area below the last track. */
export const DROP_AREA_HEIGHT = 80;

/**
 * A track being added, before its type is chosen (UI only): inserted in the layout at
 * `before` (a track of `parent`, or `null` = after the last regular track).
 */
export interface DraftTrack {
  parent: TrackId | null;
  before: TrackId | null;
}

/** Id of the draft row's stand-in track (never sent to the engine). */
export const DRAFT_TRACK_ID = "__draft_track__";

export interface Row {
  /** For the draft row, a stand-in "Return" track: nothing drops or moves onto it. */
  track: Track;
  /** Set on the draft row (see `DraftTrack`). */
  draft?: DraftTrack;
  depth: number;
  /** Top of the row in content px (0 = first row). */
  y: number;
  /** Lane height (clips live here). */
  laneHeight: number;
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
  draft: DraftTrack | null = null,
): Row[] {
  const byId = new Map(ordered.map((t) => [t.id, t]));
  const tracks = arrangementTracks(ordered, folded);
  let at = -1;
  if (draft) {
    at = draft.before ? tracks.findIndex((t) => t.id === draft.before) : -1;
    if (at < 0) at = tracks.findIndex((t) => t.kind === "Return" || t.kind === "Master");
    if (at < 0) at = tracks.length;
  }
  let y = 0;
  const rows: Row[] = [];
  const pushDraft = () => {
    rows.push(draftRow(draft!, y, byId));
    y += TRACK_HEIGHT;
  };
  tracks.forEach((track, i) => {
    if (i === at) pushDraft();
    const laneHeight = Math.round(laneHeightOf(track.id));
    const height = laneHeight + automationHeight(track.id);
    rows.push({ track, depth: depthOf(track, byId), y, laneHeight, height });
    y += height;
  });
  if (at === tracks.length) pushDraft();
  return rows;
}

function draftRow(draft: DraftTrack, y: number, byId: ReadonlyMap<TrackId, Track>): Row {
  const track = {
    id: DRAFT_TRACK_ID,
    kind: "Return",
    name: "New track",
    color: 0x8a91a8,
    order: "",
    parent: draft.parent,
    mixer: { volume: 0, pan: 0, mute: false, solo: false },
  } as unknown as Track;
  const parent = draft.parent ? byId.get(draft.parent) : undefined;
  const depth = parent ? depthOf(parent, byId) + 1 : 0;
  return { track, draft, depth, y, laneHeight: TRACK_HEIGHT, height: TRACK_HEIGHT };
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
    out.push({ id: c.id, rect: { x0, x1: x0 + c.length * vp.pxPerBeat, y0: row.y, y1: row.y + row.laneHeight } });
  }
  return out;
}
