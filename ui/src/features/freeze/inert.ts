/**
 * Group freeze (Freeze Group): the tracks inside a frozen group are "inert". The engine does
 * not process them (no clips, devices, sends or solo) and the controller rejects edits to
 * them; the UI greys them out and disables their mute/solo.
 */

import { t } from "@/i18n";
import type { Project, Track, TrackId } from "@/generated";
import { useProjectStore } from "@/state";

type Tracks = Project["tracks"];

/** The nearest frozen ancestor group of `id` (the track that makes `id` inert), if any. */
export function frozenAncestor(tracks: Tracks, id: TrackId): Track | undefined {
  let cur = tracks[id]?.parent ?? null;
  // Bounded by the track count: a corrupt parent cycle cannot loop forever.
  for (let i = 0, n = Object.keys(tracks).length; cur && i <= n; i++) {
    const g: Track | undefined = tracks[cur];
    if (!g) return undefined;
    if (g.freeze) return g;
    cur = g.parent ?? null;
  }
  return undefined;
}

/** `track` plays a render: it is frozen, or inside a frozen group. */
export function isFrozenTrack(project: Pick<Project, "tracks">, track: Track): boolean {
  return !!track.freeze || !!frozenAncestor(project.tracks, track.id);
}

/** All descendants of `group` (children, their children, ...). */
export function descendantsOf(tracks: Tracks, group: TrackId): Track[] {
  const out: Track[] = [];
  const stack = [group];
  while (stack.length) {
    const g = stack.pop();
    for (const t of Object.values(tracks)) {
      if (t.parent === g && out.length <= Object.keys(tracks).length) {
        out.push(t);
        stack.push(t.id);
      }
    }
  }
  return out;
}

/** The frozen group that makes `id` inert (re-renders when it changes). */
export function useFrozenAncestor(id: TrackId): Track | undefined {
  // Returns the group's own object: stable while that track does not change.
  return useProjectStore((s) => (s.project ? frozenAncestor(s.project.tracks, id) : undefined));
}

/** Tooltip of a control disabled because `group` (a frozen group) covers the track. */
export function inertTitle(group: Track): string {
  return t("freeze.inert_title", { name: group.name });
}
