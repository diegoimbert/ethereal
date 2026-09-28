/**
 * Pure helpers for groups, buses, track input taps and VCAs (v0.2, `groups-buses`;
 * CONTRACTS.md §12.10). No stores, no transport: the actions and views feed them the
 * project's track table.
 */

import type { Command, InputTap, Project, Track, TrackId, TrackInput } from "@/generated";
import { compareOrderKeys } from "@/state";
import { cmd } from "@/transport";

export type Tracks = Project["tracks"];

const byOrder = (a: Track, b: Track) => compareOrderKeys(a.order, b.order) || compareOrderKeys(a.id, b.id);

/** Direct children of `parent` (`null` = top level), in order. */
export function childrenOf(tracks: Tracks, parent: TrackId | null): Track[] {
  return Object.values(tracks)
    .filter((t) => t.parent === parent)
    .sort(byOrder);
}

/** Every track in display order (depth-first), with its depth. */
export function displayOrder(tracks: Tracks): Track[] {
  const out: Track[] = [];
  const walk = (parent: TrackId | null) => {
    for (const t of childrenOf(tracks, parent)) {
      out.push(t);
      walk(t.id);
    }
  };
  walk(null);
  return out;
}

/** True if `id` is `ancestor` or nested (at any depth) inside it. */
export function isWithin(tracks: Tracks, id: TrackId, ancestor: TrackId): boolean {
  let cur: TrackId | null = id;
  for (let i = 0; cur !== null && i < 256; i++) {
    if (cur === ancestor) return true;
    cur = tracks[cur]?.parent ?? null;
  }
  return false;
}

/** Tracks that can go inside a group (not master, returns or VCAs). */
export function isGroupable(t: Track): boolean {
  return t.kind === "Audio" || t.kind === "Midi" || t.kind === "Group";
}

/**
 * What Cmd+G groups: the groupable selected tracks without those already inside another
 * selected track, in display order.
 */
export function groupableSelection(tracks: Tracks, selected: Iterable<TrackId>): Track[] {
  const ids = new Set(selected);
  const insideSelected = (t: Track) => {
    let p = t.parent;
    for (let i = 0; p !== null && i < 256; i++) {
      if (ids.has(p)) return true;
      p = tracks[p]?.parent ?? null;
    }
    return false;
  };
  return displayOrder(tracks).filter((t) => ids.has(t.id) && isGroupable(t) && !insideSelected(t));
}

/**
 * The command grouping `selected` into a new group `group` (one undo step), or `null` when
 * nothing can be grouped. Tracks from different groups first move next to the first one
 * (Ableton groups whatever is selected), then `Track::GroupSelected` wraps them.
 */
export function groupCommand(tracks: Tracks, selected: Iterable<TrackId>, group: TrackId, name: string | null = null): Command | null {
  const picked = groupableSelection(tracks, selected);
  const first = picked[0];
  if (!first) return null;
  const moves: Command[] = [];
  // Strays go right after the last track already in place, in display order.
  const sibs = childrenOf(tracks, first.parent);
  let anchor = first.id;
  for (const t of picked) {
    if (t.parent === first.parent) {
      anchor = t.id;
      continue;
    }
    const at = sibs.findIndex((s) => s.id === anchor);
    const before = sibs.slice(at + 1).find((s) => !picked.some((p) => p.id === s.id))?.id ?? null;
    moves.push(cmd("Track", { type: "Move", id: t.id, parent: first.parent, before }));
  }
  const wrap = cmd("Track", { type: "GroupSelected", ids: picked.map((t) => t.id), group, name });
  if (moves.length === 0) return wrap;
  return cmd("Edit", { type: "Batch", label: "Group Tracks", commands: [...moves, wrap] });
}

/** VCA tracks, in order. */
export function vcaTracks(tracks: Tracks): Track[] {
  return Object.values(tracks)
    .filter((t) => t.kind === "Vca")
    .sort(byOrder);
}

/** VCAs `track` can be assigned to (no cycles; master never). */
export function vcaTargets(tracks: Tracks, track: Track): Track[] {
  if (track.kind === "Master") return [];
  return vcaTracks(tracks).filter((v) => {
    if (v.id === track.id) return false;
    // `v` must not be controlled (directly or not) by `track`.
    let cur: TrackId | null = v.id;
    for (let i = 0; cur !== null && i < 256; i++) {
      if (cur === track.id) return false;
      cur = tracks[cur]?.vca ?? null;
    }
    return true;
  });
}

/** Tracks directly assigned to `vca`. */
export function assignedTo(tracks: Tracks, vca: TrackId): Track[] {
  return displayOrder(tracks).filter((t) => t.vca === vca);
}

/** Tracks that can feed `consumer` as `TrackInput::Track` (no cycles, no VCAs/itself). */
export function inputSources(tracks: Tracks, consumer: Track): Track[] {
  // Everything downstream of the consumer (its outputs, parents, sends and tap consumers)
  // would form a cycle: exclude tracks that the consumer (transitively) feeds.
  const feeds = new Map<TrackId, Set<TrackId>>();
  const edge = (from: TrackId, to: TrackId) => {
    if (!feeds.has(from)) feeds.set(from, new Set());
    feeds.get(from)!.add(to);
  };
  const master = Object.values(tracks).find((t) => t.kind === "Master");
  for (const t of Object.values(tracks)) {
    if (t.output.type === "Track") edge(t.id, t.output.track);
    else if (t.output.type === "Default" && t.kind !== "Master") {
      const to = t.parent ?? master?.id;
      if (to) edge(t.id, to);
    }
    if (t.input.type === "Track") edge(t.input.track, t.id);
  }
  const downstream = new Set<TrackId>([consumer.id]);
  const stack = [consumer.id];
  while (stack.length > 0) {
    const id = stack.pop()!;
    for (const next of feeds.get(id) ?? []) {
      if (!downstream.has(next)) {
        downstream.add(next);
        stack.push(next);
      }
    }
  }
  return displayOrder(tracks).filter((t) => t.kind !== "Vca" && t.kind !== "Master" && !downstream.has(t.id));
}

export const INPUT_TAPS: ReadonlyArray<{ tap: InputTap; label: string; short: string }> = [
  { tap: "PreFx", label: "Pre FX", short: "Pre" },
  { tap: "PostFx", label: "Post FX", short: "FX" },
  { tap: "PostFader", label: "Post Mixer", short: "Post" },
];

export function tapLabel(tap: InputTap): string {
  return INPUT_TAPS.find((t) => t.tap === tap)?.label ?? tap;
}

/** `TrackInput::Track` from `source` at `tap`. */
export function trackInput(source: TrackId, tap: InputTap = "PostFader"): TrackInput {
  return { type: "Track", track: source, tap };
}

/** What ungrouping `group` would lose (mirrors the controller's `InvalidState` check). */
export function ungroupLosses(project: Pick<Project, "tracks" | "devices" | "automation_lanes" | "sends">, group: TrackId): string[] {
  const lost: string[] = [];
  const devices = Object.values(project.devices).filter((d) => d.track === group && d.pad === null && d.chain == null).length;
  if (devices > 0) lost.push(devices === 1 ? "1 device" : `${devices} devices`);
  const lanes = Object.values(project.automation_lanes).filter(
    (l) =>
      (l.owner.type === "Track" && l.owner.track === group) ||
      ((l.target.type === "TrackVolume" || l.target.type === "TrackPan") && l.target.track === group),
  ).length;
  if (lanes > 0) lost.push(lanes === 1 ? "1 automation lane" : `${lanes} automation lanes`);
  const sends = Object.values(project.sends).filter((s) => s.from === group || s.to === group).length;
  if (sends > 0) lost.push(sends === 1 ? "1 send" : `${sends} sends`);
  const routed = Object.values(project.tracks).filter(
    (t) => (t.output.type === "Track" && t.output.track === group) || (t.input.type === "Track" && t.input.track === group),
  ).length;
  if (routed > 0) lost.push(routed === 1 ? "1 routing to it" : `${routed} routings to it`);
  return lost;
}

/** Tracks that take a track-to-track input in the mixer (audio tracks: they can record it). */
export function takesTrackInput(track: Track): boolean {
  return track.kind === "Audio";
}
