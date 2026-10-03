/**
 * Derived views of the normalized `Project` (pure functions) and React hooks over the
 * project store.
 *
 * The pure functions allocate new arrays; the hooks wrap them in `useShallow` so a
 * component only re-renders when the selected entities actually change (immer keeps
 * untouched entity objects referentially stable across patches).
 */

import { useShallow } from "zustand/react/shallow";
import type {
  AutomationLane,
  AutomationLaneId,
  AutomationPoint,
  Clip,
  ClipId,
  Device,
  DrumPadId,
  Note,
  Project,
  TempoPoint,
  TimeSignaturePoint,
  Track,
  TrackId,
  TrackSend,
  WarpMarker,
} from "@/generated";
import { compareOrderKeys } from "./orderKey";
import { useProjectStore } from "./projectStore";

const byOrder = (a: { order: string; id: string }, b: { order: string; id: string }) =>
  compareOrderKeys(a.order, b.order) || compareOrderKeys(a.id, b.id);

const byTime = (a: { time: number; id: string }, b: { time: number; id: string }) =>
  a.time - b.time || compareOrderKeys(a.id, b.id);

/** Direct children of `parent` (`null` = top level), sorted by `order`. */
export function childTracks(project: Project, parent: TrackId | null): Track[] {
  return Object.values(project.tracks)
    .filter((t) => t.parent === parent)
    .sort(byOrder);
}

/**
 * Every track in display order: top-level tracks by `order`, each group immediately
 * followed by its children (depth-first). Includes return and master tracks; filter by
 * `kind` as needed.
 */
export function tracksOrdered(project: Project): Track[] {
  const out: Track[] = [];
  const visit = (parent: TrackId | null) => {
    for (const t of childTracks(project, parent)) {
      out.push(t);
      if (t.kind === "Group") visit(t.id);
    }
  };
  visit(null);
  return out;
}

/** Nesting depth of a track (0 = top level). */
export function trackDepth(project: Project, track: TrackId): number {
  let depth = 0;
  let parent = project.tracks[track]?.parent ?? null;
  while (parent !== null && depth < 64) {
    depth++;
    parent = project.tracks[parent]?.parent ?? null;
  }
  return depth;
}

export function masterTrack(project: Project): Track | undefined {
  return Object.values(project.tracks).find((t) => t.kind === "Master");
}

/**
 * Entities of a table grouped by their owner (notes by clip, clips by track, ...), each
 * group sorted. Built once per table object: the store is immutable (immer), so a table
 * changes identity exactly when it changes, and the index is rebuilt on the next read.
 * Lookups are O(1) and return the same array until the table changes, so components
 * re-render only when their own entities did. Mutable tables (the mock engine's working
 * copy) aren't frozen and are grouped fresh on every call instead.
 */
function groupedBy<T, K>(
  cache: WeakMap<object, Map<K, T[]>>,
  table: Readonly<Record<string, T>>,
  keyOf: (t: T) => K | null,
  compare: (a: T, b: T) => number,
): Map<K, T[]> {
  const hit = cache.get(table);
  if (hit) return hit;
  const groups = new Map<K, T[]>();
  for (const t of Object.values(table)) {
    const k = keyOf(t);
    if (k === null) continue;
    const g = groups.get(k);
    if (g) g.push(t);
    else groups.set(k, [t]);
  }
  for (const g of groups.values()) g.sort(compare);
  if (Object.isFrozen(table)) {
    // Shared between callers: never mutate a returned group (copy it first).
    for (const g of groups.values()) Object.freeze(g);
    cache.set(table, groups);
  }
  return groups;
}

const NONE: never[] = Object.freeze([]) as never[];
const notesByClip = new WeakMap<object, Map<ClipId, Note[]>>();
const clipsByTrack = new WeakMap<object, Map<TrackId, Clip[]>>();
const devicesByTrack = new WeakMap<object, Map<TrackId, Device[]>>();
const pointsByLane = new WeakMap<object, Map<AutomationLaneId, AutomationPoint[]>>();
const markersByClip = new WeakMap<object, Map<ClipId, WarpMarker[]>>();

/** The track's own device chain (devices on drum pads are excluded: see `devicesOfPad`). */
export function devicesOfTrack(project: Project, track: TrackId): Device[] {
  return groupedBy(devicesByTrack, project.devices, (d) => (d.pad === null && d.chain == null ? d.track : null), byOrder).get(track) ?? NONE;
}

/** The device chain of a drum pad (mirrors `Project::pad_devices_of`). */
export function devicesOfPad(project: Project, pad: DrumPadId): Device[] {
  return Object.values(project.devices)
    .filter((d) => d.pad === pad)
    .sort(byOrder);
}

/** Arrangement clips of a track, sorted by start. */
export function clipsOfTrack(project: Project, track: TrackId): Clip[] {
  return (
    groupedBy(clipsByTrack, project.clips, (c) => c.track, (a, b) => a.start - b.start || compareOrderKeys(a.id, b.id)).get(
      track,
    ) ?? NONE
  );
}

/** Notes of a MIDI clip, sorted by start then pitch. */
export function notesOfClip(project: Project, clip: ClipId): Note[] {
  const order = (a: Note, b: Note) => a.start - b.start || a.pitch - b.pitch || compareOrderKeys(a.id, b.id);
  return groupedBy(notesByClip, project.notes, (n) => n.clip, order).get(clip) ?? NONE;
}

/** Breakpoints of an automation lane, sorted by time. */
export function pointsOfLane(project: Project, lane: AutomationLaneId): AutomationPoint[] {
  return groupedBy(pointsByLane, project.automation_points, (p) => p.lane, byTime).get(lane) ?? NONE;
}

/** Arrangement automation lanes of a track. */
export function lanesOfTrack(project: Project, track: TrackId): AutomationLane[] {
  return Object.values(project.automation_lanes).filter((l) => l.owner.type === "Track" && l.owner.track === track);
}

/** Clip envelopes of a clip. */
export function lanesOfClip(project: Project, clip: ClipId): AutomationLane[] {
  return Object.values(project.automation_lanes).filter((l) => l.owner.type === "Clip" && l.owner.clip === clip);
}

/** Sends out of a track. */
export function sendsOfTrack(project: Project, track: TrackId): TrackSend[] {
  return Object.values(project.sends).filter((s) => s.from === track);
}

export function warpMarkersOfClip(project: Project, clip: ClipId): WarpMarker[] {
  return groupedBy(markersByClip, project.warp_markers, (m) => m.clip, (a, b) => a.beat - b.beat).get(clip) ?? NONE;
}

export function tempoPoints(project: Project): TempoPoint[] {
  return Object.values(project.tempo_points).sort(byTime);
}

export function timeSignaturePoints(project: Project): TimeSignaturePoint[] {
  return Object.values(project.time_signatures).sort(byTime);
}

// ─── Hooks ──────────────────────────────────────────────────────────────────────────────

const EMPTY: never[] = [];

/** Select a derived list from the current project with shallow equality. */
function useProjectList<T>(select: (project: Project) => T[]): T[] {
  return useProjectStore(useShallow((s) => (s.project ? select(s.project) : EMPTY)));
}

export const useProject = (): Project | null => useProjectStore((s) => s.project);
export const useTrack = (id: TrackId | null | undefined): Track | undefined =>
  useProjectStore((s) => (id ? s.project?.tracks[id] : undefined));
export const useClip = (id: ClipId | null | undefined): Clip | undefined =>
  useProjectStore((s) => (id ? s.project?.clips[id] : undefined));

export const useTracksOrdered = (): Track[] => useProjectList(tracksOrdered);
export const useDevicesOfTrack = (track: TrackId): Device[] => useProjectList((p) => devicesOfTrack(p, track));
export const useClipsOfTrack = (track: TrackId): Clip[] => useProjectList((p) => clipsOfTrack(p, track));
export const useNotesOfClip = (clip: ClipId): Note[] => useProjectList((p) => notesOfClip(p, clip));
export const usePointsOfLane = (lane: AutomationLaneId): AutomationPoint[] =>
  useProjectList((p) => pointsOfLane(p, lane));
