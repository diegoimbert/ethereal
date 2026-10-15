/**
 * Pure helpers: the time selection a marquee drag makes, and the `TimeEdit` commands for
 * the selection (CONTRACTS.md §12.3).
 */

import type { Beats, Clip, ClipId, Project, TimeSelection, Track, TrackId } from "@/generated";
import { tracksOrdered } from "@/state";
import type { Rect } from "@/timeline";
import type { TimeRangeSelection } from "./store";

/** Tracks a time selection can span (clips or children): audio, MIDI and group tracks. */
export function isTimeTrack(t: Pick<Track, "kind">): boolean {
  return t.kind === "Audio" || t.kind === "Midi" || t.kind === "Group";
}

/** What the time-selection code needs of an arrangement row. */
export interface SelectionRow {
  track: Pick<Track, "id" | "kind">;
  y: number;
  height: number;
  draft?: unknown;
}

/**
 * The time selection of a marquee drag over the lanes: `rect` in content px (lanes start at
 * `headerWidth`), beats through `toBeats` (snapped by the caller). `null` if the drag spans
 * no audio/MIDI/group row or no time.
 */
export function selectionFromRect(
  rect: Rect,
  rows: ReadonlyArray<SelectionRow>,
  headerWidth: number,
  toBeats: (lanePx: number) => Beats,
): TimeRangeSelection | null {
  const start = Math.max(0, toBeats(Math.max(0, rect.x0 - headerWidth)));
  const end = Math.max(0, toBeats(Math.max(0, rect.x1 - headerWidth)));
  const tracks = rows.filter((r) => isTimeTrack(r.track) && r.y < rect.y1 && r.y + r.height > rect.y0).map((r) => r.track.id);
  if (!tracks.length || end - start <= 1e-6) return null;
  return { start, end, tracks };
}

/**
 * The audio/MIDI/group tracks a selection of `tracks` applies to: the listed ones and the
 * descendants of listed groups, in display order (like the engine).
 */
export function expandTracks(project: Project, tracks: ReadonlyArray<TrackId>): TrackId[] {
  const covered = new Set<TrackId>(tracks);
  const all = tracksOrdered(project);
  // Parents come before their children in display order.
  for (const t of all) if (t.parent !== null && covered.has(t.parent)) covered.add(t.id);
  return all.filter((t) => isTimeTrack(t) && covered.has(t.id)).map((t) => t.id);
}

/**
 * `true` if `tracks` cover every audio, MIDI and group track: the whole song is selected,
 * so a time edit also moves markers, tempo and time signatures (`global`).
 */
export function coversWholeSong(project: Project, tracks: ReadonlyArray<TrackId>): boolean {
  const covered = new Set(expandTracks(project, tracks));
  return tracksOrdered(project)
    .filter(isTimeTrack)
    .every((t) => covered.has(t.id));
}

/** The protocol selection of a time selection. */
export function timeSelection(project: Project, sel: TimeRangeSelection): TimeSelection {
  return {
    start: sel.start,
    end: sel.end,
    tracks: [...sel.tracks],
    global: coversWholeSong(project, sel.tracks),
  };
}

/**
 * Where a paste lands: the audio/MIDI/group tracks from `first` on, in display order (the
 * engine maps copied track `i` to the `i`-th one and skips kind mismatches). `first: null`
 * = the copied tracks.
 */
export function pasteTracks(project: Project, first: TrackId | null): TrackId[] {
  if (first === null) return [];
  const order = tracksOrdered(project).filter(isTimeTrack);
  const i = order.findIndex((t) => t.id === first);
  return i < 0 ? [] : order.slice(i).map((t) => t.id);
}

/** A time span on some tracks (audio, MIDI and group tracks, in display order). */
export interface TimeSpan {
  start: Beats;
  end: Beats;
  tracks: TrackId[];
}

/** The span of the given arrangement clips (earliest start to latest end, on their tracks), or null. */
export function clipSpan(project: Project, clips: Iterable<ClipId>): TimeSpan | null {
  const picked = [...clips].map((id) => project.clips[id]).filter((c): c is Clip => c !== undefined && c.lane == null);
  if (!picked.length) return null;
  const start = Math.min(...picked.map((c) => c.start));
  const end = Math.max(...picked.map((c) => c.start + c.length));
  const on = new Set(picked.map((c) => c.track));
  const tracks = tracksOrdered(project)
    .filter((t) => isTimeTrack(t) && on.has(t.id))
    .map((t) => t.id);
  return end - start > 1e-6 && tracks.length ? { start, end, tracks } : null;
}
