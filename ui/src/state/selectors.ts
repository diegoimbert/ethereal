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
import {
  NONE,
  byOrder,
  byTime,
  clipOrder,
  clipsByTrack,
  devicesByTrack,
  groupedBy,
  markersByClip,
  noteOrder,
  notesByClip,
  pointsByLane,
  trackChainKey,
} from "./groups";
import { useProjectStore } from "./projectStore";

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

/** The track's own device chain (devices on drum pads are excluded: see `devicesOfPad`). */
export function devicesOfTrack(project: Project, track: TrackId): Device[] {
  return groupedBy(devicesByTrack, project.devices, trackChainKey, byOrder).get(track) ?? NONE;
}

/** The device chain of a drum pad (mirrors `Project::pad_devices_of`). */
export function devicesOfPad(project: Project, pad: DrumPadId): Device[] {
  return Object.values(project.devices)
    .filter((d) => d.pad === pad)
    .sort(byOrder);
}

/** Arrangement clips of a track, sorted by start. */
export function clipsOfTrack(project: Project, track: TrackId): Clip[] {
  return groupedBy(clipsByTrack, project.clips, (c) => c.track, clipOrder).get(track) ?? NONE;
}

/** Notes of a MIDI clip, sorted by start then pitch. */
export function notesOfClip(project: Project, clip: ClipId): Note[] {
  return groupedBy(notesByClip, project.notes, (n) => n.clip, noteOrder).get(clip) ?? NONE;
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
  return (
    groupedBy(
      markersByClip,
      project.warp_markers,
      (m) => m.clip,
      (a, b) => a.beat - b.beat,
    ).get(clip) ?? NONE
  );
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
export const useTrack = (id: TrackId | null | undefined): Track | undefined => useProjectStore((s) => (id ? s.project?.tracks[id] : undefined));
export const useClip = (id: ClipId | null | undefined): Clip | undefined => useProjectStore((s) => (id ? s.project?.clips[id] : undefined));

export const useTracksOrdered = (): Track[] => useProjectList(tracksOrdered);
export const useDevicesOfTrack = (track: TrackId): Device[] => useProjectList((p) => devicesOfTrack(p, track));
export const useClipsOfTrack = (track: TrackId): Clip[] => useProjectList((p) => clipsOfTrack(p, track));
export const useNotesOfClip = (clip: ClipId): Note[] => useProjectList((p) => notesOfClip(p, clip));
export const usePointsOfLane = (lane: AutomationLaneId): AutomationPoint[] => useProjectList((p) => pointsOfLane(p, lane));
