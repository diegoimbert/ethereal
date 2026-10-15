/** Moving a track one place up or down among its siblings (the track sheet's Move up / down). */
import type { Command, Project, Track, TrackId } from "@/generated";
import { tracksOrdered } from "@/state";
import { cmd } from "@/transport";

/** The list a track reorders in: the regular tracks, or the returns (master never moves). */
const section = (t: Track) => (t.kind === "Master" ? null : t.kind === "Return" ? "returns" : "tracks");

/** The command moving `id` one place up (-1) or down (+1) among its siblings, or null at an end. */
export function siblingMove(project: Project, id: TrackId, dir: -1 | 1): Command | null {
  const track = project.tracks[id];
  const own = track ? section(track) : null;
  if (!track || !own) return null;
  const siblings = tracksOrdered(project).filter((t) => t.parent === track.parent && section(t) === own);
  const i = siblings.findIndex((t) => t.id === id);
  if (i < 0) return null;
  if (dir === -1) {
    if (i === 0) return null;
    return cmd("Track", { type: "Move", id, parent: track.parent, before: siblings[i - 1]!.id });
  }
  if (i === siblings.length - 1) return null;
  // `before: null` at the end: the engine puts a regular track before the returns, a return before master.
  return cmd("Track", { type: "Move", id, parent: track.parent, before: siblings[i + 2]?.id ?? null });
}
