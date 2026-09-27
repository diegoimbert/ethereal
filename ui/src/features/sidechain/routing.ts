/**
 * Which tracks can feed a device's sidechain (mirrors the controller's validation in
 * `crates/ether-controller/src/sidechain/mod.rs` + the model's routing-cycle check), so the
 * selector can disable choices the engine would reject.
 */

import type { Device, Project, Track, TrackId } from "@/generated";
import { tracksOrdered } from "@/state";

/** Routing edges: outputs (default = parent group or master), sends, sidechains. */
function edges(project: Project): Map<TrackId, TrackId[]> {
  const tracks = Object.values(project.tracks);
  const master = tracks.find((t) => t.kind === "Master")?.id ?? null;
  const out = new Map<TrackId, TrackId[]>();
  const add = (a: TrackId, b: TrackId) => out.set(a, [...(out.get(a) ?? []), b]);
  for (const t of tracks) {
    if (t.output.type === "Track") add(t.id, t.output.track);
    else if (t.output.type === "Default" && t.kind !== "Master") {
      const dest = t.parent ?? master;
      if (dest !== null) add(t.id, dest);
    }
  }
  for (const s of Object.values(project.sends)) add(s.from, s.to);
  for (const d of Object.values(project.devices)) if (d.sidechain !== null) add(d.sidechain, d.track);
  return out;
}

function reaches(graph: Map<TrackId, TrackId[]>, from: TrackId, to: TrackId): boolean {
  const seen = new Set<TrackId>();
  const stack = [from];
  while (stack.length > 0) {
    const t = stack.pop()!;
    if (t === to) return true;
    if (seen.has(t)) continue;
    seen.add(t);
    stack.push(...(graph.get(t) ?? []));
  }
  return false;
}

export interface SourceOption {
  track: Track;
  /** Selecting it would close a routing cycle (the engine rejects it). */
  cycle: boolean;
}

/** Candidate sources for `device`, in display order: every track but master and its own. */
export function sidechainSources(project: Project, device: Device): SourceOption[] {
  const graph = edges(project);
  return tracksOrdered(project)
    .filter((t) => t.kind !== "Master" && t.id !== device.track)
    .map((track) => ({
      track,
      // The edge track → device.track closes a cycle iff track is reachable from it.
      cycle: track.id !== device.sidechain && reaches(graph, device.track, track.id),
    }));
}
