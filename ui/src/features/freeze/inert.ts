/**
 * Group freeze (Freeze Group): the tracks inside a frozen group are "inert". The engine does
 * not process them (no clips, devices, sends or solo) and the controller rejects edits to
 * them; the UI greys them out and disables their mute/solo.
 */

import { t } from "@/i18n";
import type { Device, DeviceId, Project, Track, TrackId } from "@/generated";
import { devicesOfTrack, useProjectStore } from "@/state";

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

/**
 * The track-chain device `device` sits under: itself on a track chain, else the rack (drum
 * rack or chain rack, nested racks followed up) holding it (mirrors `Project::chain_root`).
 */
export function chainRoot(project: Pick<Project, "devices" | "drum_pads" | "rack_chains">, device: Device): Device | undefined {
  let d: Device | undefined = device;
  for (let i = 0, n = Object.keys(project.devices).length; d && i <= n; i++) {
    if (!d.pad && !d.chain) return d;
    const rack: DeviceId | undefined = d.pad ? project.drum_pads[d.pad]?.rack : d.chain ? project.rack_chains[d.chain]?.rack : undefined;
    d = rack ? project.devices[rack] : undefined;
  }
  return undefined;
}

/**
 * How many devices of `track`'s chain are frozen (mirrors `Project::frozen_chain_len`):
 * `null` when the track is not frozen; the whole chain for a plain freeze or inside a frozen
 * group; else up to and including the freeze point (`TrackFreeze::up_to`, "freeze up to
 * here": the devices after it stay live and editable).
 */
export function frozenChainLength(project: Project, track: TrackId): number | null {
  const t = project.tracks[track];
  if (!t) return null;
  const chain = devicesOfTrack(project, track);
  if (!t.freeze) return frozenAncestor(project.tracks, track) ? chain.length : null;
  const at = t.freeze.up_to ? chain.findIndex((d) => d.id === t.freeze!.up_to) : -1;
  return at < 0 ? chain.length : at + 1;
}

/** `device` is frozen: at or before its track's freeze point (nested devices follow their rack). */
export function isDeviceFrozen(project: Project, device: Device): boolean {
  const root = chainRoot(project, device);
  if (!root) return false;
  const n = frozenChainLength(project, root.track);
  if (n === null) return false;
  const at = devicesOfTrack(project, root.track).findIndex((d) => d.id === root.id);
  return at < 0 || at < n;
}

/** Whether `device` is frozen (re-renders when it changes). */
export function useDeviceFrozen(device: Device): boolean {
  return useProjectStore((s) => !!s.project && isDeviceFrozen(s.project, device));
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
