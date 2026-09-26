/** Pure helpers for the mixer: layout order, output routing choices and fader law. */

import type { Project, Track, TrackId, TrackOutput } from "@/generated";
import { compareOrderKeys } from "@/state";
import { scaleToNormalized, scaleToPlain, SILENCE_DB } from "@/features/devices/paramScale";

/** Top of the volume/send fader range (`Decibels` max accepted by the engine). */
export const MAX_DB = 6;

const FADER = { type: "Fader" } as const;

/** Fader position (0..1) → dB, with the protocol's `ParamScale::Fader` law. */
export function faderToDb(n: number, max = MAX_DB): number {
  return scaleToPlain(FADER, SILENCE_DB, max, n);
}

/** dB → fader position (0..1). */
export function dbToFader(db: number, max = MAX_DB): number {
  return scaleToNormalized(FADER, SILENCE_DB, max, db);
}

/** The track table of a project. */
export type Tracks = Project["tracks"];

/** Direct children of `parent` (`null` = top level), in order. */
export function childTracks(tracks: Tracks, parent: TrackId | null): Track[] {
  return Object.values(tracks)
    .filter((t) => t.parent === parent)
    .sort((a, b) => compareOrderKeys(a.order, b.order) || compareOrderKeys(a.id, b.id));
}

/** A node of the strip layout: a track plus (for groups) its nested children. */
export interface StripNode {
  track: Track;
  children: StripNode[];
}

/**
 * Mixer layout (Ableton order): regular tracks as a tree (groups contain their children),
 * then return tracks, then master.
 */
export function mixerLayout(tracks: Tracks): { tracks: StripNode[]; returns: Track[]; master: Track | undefined } {
  const build = (parent: TrackId | null): StripNode[] =>
    childTracks(tracks, parent)
      .filter((t) => t.kind !== "Return" && t.kind !== "Master")
      .map((t) => ({ track: t, children: t.kind === "Group" ? build(t.id) : [] }));
  const top = childTracks(tracks, null);
  return {
    tracks: build(null),
    returns: top.filter((t) => t.kind === "Return"),
    master: top.find((t) => t.kind === "Master"),
  };
}

/** True if `id` is `ancestor` or nested (at any depth) inside it. */
export function isWithin(tracks: Tracks, id: TrackId, ancestor: TrackId): boolean {
  let cur: TrackId | null = id;
  for (let i = 0; cur !== null && i < 64; i++) {
    if (cur === ancestor) return true;
    cur = tracks[cur]?.parent ?? null;
  }
  return false;
}

/**
 * Tracks `track` may output to: group and return tracks, except itself and (for a group)
 * its own descendants, which would form a cycle.
 */
export function outputTargets(tracks: Tracks, track: Track): Track[] {
  const out: Track[] = [];
  const visit = (parent: TrackId | null) => {
    for (const t of childTracks(tracks, parent)) {
      if ((t.kind === "Group" || t.kind === "Return") && !isWithin(tracks, t.id, track.id)) out.push(t);
      if (t.kind === "Group") visit(t.id);
    }
  };
  visit(null);
  return out;
}

/** Label of `TrackOutput::Default` for this track: its parent group bus, else master. */
export function defaultOutputLabel(tracks: Tracks, track: Track): string {
  const parent = track.parent ? tracks[track.parent] : undefined;
  return parent ? `Group (${parent.name})` : "Master";
}

/** `<select>` value encoding of a `TrackOutput`. */
export function outputValue(output: TrackOutput): string {
  switch (output.type) {
    case "Default":
      return "default";
    case "None":
      return "none";
    case "Track":
      return `track:${output.track}`;
  }
}

export function parseOutputValue(value: string): TrackOutput {
  if (value === "none") return { type: "None" };
  if (value.startsWith("track:")) return { type: "Track", track: value.slice("track:".length) };
  return { type: "Default" };
}
