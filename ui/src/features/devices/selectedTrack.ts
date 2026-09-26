import type { Project, Track, TrackId } from "@/generated";
import { tracksOrdered, useProjectStore, useSelectedTrackId } from "@/state";

/**
 * The selected track, or (no selection / a stale id) the first non-master track, then
 * master.
 */
export function resolveSelectedTrack(project: Project, id: TrackId | null): Track | undefined {
  const t = id ? project.tracks[id] : undefined;
  if (t) return t;
  const ordered = tracksOrdered(project);
  return ordered.find((x) => x.kind !== "Master") ?? ordered[0];
}

export function useSelectedTrack(): Track | undefined {
  const id = useSelectedTrackId();
  return useProjectStore((s) => (s.project ? resolveSelectedTrack(s.project, id) : undefined));
}
