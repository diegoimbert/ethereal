/** Group membership rails: a track inside groups shows one rail per enclosing group (TrackRow). */

import type { Track, TrackId } from "@/generated";
import { useProjectStore } from "@/state";
import { colorCss } from "./helpers";

/** The groups enclosing `track`, outermost first (empty at the top level). */
export function groupAncestors(track: Track, tracks: Readonly<Record<TrackId, Track | undefined>>): Track[] {
  const out: Track[] = [];
  let parent = track.parent;
  for (let depth = 0; parent && depth < 64; depth++) {
    const group = tracks[parent];
    if (!group) break;
    out.unshift(group);
    parent = group.parent;
  }
  return out;
}

/** The CSS colors of the groups enclosing `track`, outermost first. */
export function useGroupRailColors(track: Track): string[] {
  // One string, so the row re-renders only when a color (or the nesting) changes.
  const key = useProjectStore((s) => {
    const tracks = s.project?.tracks;
    return tracks
      ? groupAncestors(track, tracks)
          .map((g) => colorCss(g.color))
          .join("|")
      : "";
  });
  return key ? key.split("|") : [];
}
