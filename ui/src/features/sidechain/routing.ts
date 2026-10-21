/**
 * Which sources and tap points can feed a device's sidechain (mirrors the controller's
 * validation in `crates/ether-controller/src/sidechain/mod.rs` + the model's rules in
 * `ether_model::SidechainTap` and its routing-cycle check), so the selector can disable
 * choices the engine would reject. Devices take a sidechain at any depth (track chain, drum
 * pads, rack chains); sources are a track's Pre FX / Post FX / Post Mixer, or the same points
 * of one of its rack chains or drum pads (any depth), like Live's "Audio From" choosers.
 */

import type { Device, DeviceId, InputTap, Project, SidechainChain, SidechainTap, Track, TrackId } from "@/generated";
import { t, type MessageKey } from "@/i18n";
import { compareOrderKeys, tracksOrdered } from "@/state";

type P = Pick<Project, "tracks" | "devices" | "sends" | "rack_chains" | "drum_pads">;

/** Live's names of the tap points (the group inputs' words). */
export const POINT_LABEL: Record<InputTap, MessageKey> = {
  PreFx: "groups.input.pre_fx",
  PostFx: "groups.input.post_fx",
  PostFader: "groups.input.post_mixer",
};

/** Why a tap is refused ([`tapError`]). */
export type TapError =
  | "unknown_track"
  | "master"
  | "loop"
  | "after_device"
  | "not_on_track"
  | "broken_nesting"
  | "holds_device"
  | "sibling_chain"
  | "sibling_post_fader";

/** What the selector shows for each [`TapError`]. */
export const TAP_ERROR_TEXT: Record<TapError, MessageKey> = {
  unknown_track: "sidechain.error.unknown_track",
  master: "sidechain.error.master",
  loop: "sidechain.error.loop",
  after_device: "sidechain.error.after_device",
  not_on_track: "sidechain.error.not_on_track",
  broken_nesting: "sidechain.error.broken_nesting",
  holds_device: "sidechain.error.holds_device",
  sibling_chain: "sidechain.error.sibling_chain",
  sibling_post_fader: "sidechain.error.sibling_post_fader",
};
const POINTS: InputTap[] = ["PreFx", "PostFx", "PostFader"];

/** The original sidechain point (`ether_model::SidechainTap::POST_FADER`). */
export const POST_FADER: SidechainTap = { point: "PostFader" };

/** A device's effective tap (`null`/missing = post-fader). */
export function tapOf(d: Device): SidechainTap {
  return d.sidechain_tap ?? POST_FADER;
}

const byOrder = (a: { order: string; id: string }, b: { order: string; id: string }) => compareOrderKeys(a.order, b.order) || compareOrderKeys(a.id, b.id);

/** Routing edges: outputs (default = parent group or master), sends, other-track sidechains. */
function edges(project: P): Map<TrackId, TrackId[]> {
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
  for (const d of Object.values(project.devices)) if (d.sidechain !== null && d.sidechain !== d.track) add(d.sidechain, d.track);
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

/** Whether the edge `source → device.track` (other track) would close a routing cycle. */
export function closesCycle(project: P, device: Device, source: TrackId): boolean {
  return source !== device.track && reaches(edges(project), device.track, source);
}

/** The rack holding chain `c` (a rack chain's rack, a pad's drum rack). */
function rackOf(project: P, c: SidechainChain): DeviceId | null {
  return c.type === "Rack" ? (project.rack_chains[c.chain]?.rack ?? null) : (project.drum_pads[c.pad]?.rack ?? null);
}

/** Devices from the track chain down to `id` (inclusive), outermost first. */
function pathOf(project: P, id: DeviceId): DeviceId[] | null {
  const out = [id];
  let d = project.devices[id];
  while (d) {
    const parent = d.chain != null ? project.rack_chains[d.chain]?.rack : d.pad !== null ? project.drum_pads[d.pad]?.rack : undefined;
    if (d.chain == null && d.pad === null) break;
    if (parent === undefined || out.includes(parent) || out.length > 8) return null;
    out.push(parent);
    d = project.devices[parent];
  }
  return out.reverse();
}

/**
 * The order a same-track tap sets between two chains of one rack (`Project::sibling_chain_edge`):
 * `device` keys chain `from` of `rack` (or a point inside it) from inside another chain `to`
 * of that rack, so `from` runs first. `null` for any other tap.
 */
function siblingEdge(project: P, device: Device, tap: SidechainTap): { rack: DeviceId; from: string; to: string } | null {
  if (tap.chain?.type !== "Rack") return null;
  const chain = tap.chain.chain;
  const rack = project.rack_chains[chain]?.rack;
  if (!rack) return null;
  const rp = pathOf(project, rack);
  const dp = pathOf(project, device.id);
  if (!rp || !dp) return null;
  const i = rp.findIndex((r, k) => dp[k] !== r);
  if (i < 0) {
    // The tapped chain's rack holds the device: the chain of that rack holding it.
    const to = dp[rp.length] !== undefined ? project.devices[dp[rp.length]!]?.chain : null;
    return to && to !== chain ? { rack, from: chain, to } : null;
  }
  const p = project.devices[rp[i]!];
  const q = project.devices[dp[i]!];
  if (!p?.chain || !q?.chain || p.chain === q.chain) return null;
  const r = project.rack_chains[p.chain]?.rack;
  return r && project.rack_chains[q.chain]?.rack === r ? { rack: r, from: p.chain, to: q.chain } : null;
}

/** Whether `device` keying at `tap` would make chains of one rack key each other in a loop. */
function chainsLoop(project: P, device: Device, tap: SidechainTap): boolean {
  const edge = siblingEdge(project, device, tap);
  if (!edge) return false;
  const edges = Object.values(project.devices)
    .filter((d) => d.id !== device.id && d.track === device.track && d.sidechain === device.track)
    .flatMap((d) => {
      const e = siblingEdge(project, d, tapOf(d));
      return e && e.rack === edge.rack ? [e] : [];
    });
  // A loop: `edge.to` already reaches `edge.from`.
  const seen = new Set<string>();
  const stack = [edge.to];
  while (stack.length > 0) {
    const c = stack.pop()!;
    if (c === edge.from) return true;
    if (seen.has(c)) continue;
    seen.add(c);
    stack.push(...edges.filter((e) => e.from === c).map((e) => e.to));
  }
  return false;
}

/**
 * Why `device` can't take `source` at `tap` (`null` = it can). Same rules as the model:
 * the chain must be on the source; on its own track only the Pre FX signal, a chain of a
 * rack placed before it, or another chain of a rack it is in (that chain runs first; before
 * its volume: Pre or Post FX), never in a loop; no routing cycle with other tracks.
 */
export function tapError(project: P, device: Device, source: TrackId, tap: SidechainTap): TapError | null {
  const src = project.tracks[source];
  if (!src) return "unknown_track";
  if (src.kind === "Master") return "master";
  if (closesCycle(project, device, source)) return "loop";
  if (!tap.chain) {
    return source === device.track && tap.point !== "PreFx" ? "after_device" : null;
  }
  const rack = rackOf(project, tap.chain);
  if (rack === null || project.devices[rack]?.track !== source) return "not_on_track";
  if (source !== device.track) return null;
  const rp = pathOf(project, rack);
  const dp = pathOf(project, device.id);
  if (!rp || !dp) return "broken_nesting";
  const sibling = siblingEdge(project, device, tap) !== null;
  if (sibling && chainsLoop(project, device, tap)) return "loop";
  const i = rp.findIndex((r, k) => dp[k] !== r);
  if (i < 0 || i >= dp.length) {
    if (!sibling) return "holds_device";
    return tap.point === "PostFader" ? "sibling_post_fader" : null;
  }
  const p = project.devices[rp[i]!]!;
  const q = project.devices[dp[i]!]!;
  if ((p.pad ?? null) !== (q.pad ?? null) || (p.chain ?? null) !== (q.chain ?? null)) return sibling ? null : "sibling_chain";
  return byOrder(p, q) < 0 ? null : "after_device";
}

export interface SourceOption {
  track: Track;
  /** Selecting it would close a routing cycle (the engine rejects it). */
  cycle: boolean;
}

/**
 * Candidate source tracks for `device`, in display order: every track but master (its own
 * track too: its Pre FX signal and earlier racks' chains can key it).
 */
export function sidechainSources(project: P & Project, device: Device): SourceOption[] {
  const graph = edges(project);
  return tracksOrdered(project)
    .filter((t) => t.kind !== "Master")
    .map((track) => ({
      track,
      // The edge track → device.track closes a cycle iff track is reachable from it.
      cycle: track.id !== device.sidechain && track.id !== device.track && reaches(graph, device.track, track.id),
    }));
}

/** A tap point of a source track as the selector lists it. */
export interface TapOption {
  /** Stable key ([`tapKey`]). */
  key: string;
  tap: SidechainTap;
  /** "Pre FX", or "Instrument Rack | Kick SC | Pre FX" (Live's notation). */
  label: string;
}

/** A stable string for a tap (option values). */
export function tapKey(tap: SidechainTap): string {
  if (!tap.chain) return tap.point;
  return tap.chain.type === "Rack" ? `${tap.point}|R|${tap.chain.chain}` : `${tap.point}|P|${tap.chain.pad}`;
}

/** Every tap point of `track`: the track's, then each rack chain / drum pad's (any depth). */
export function tapPoints(project: P, track: TrackId): TapOption[] {
  const out: TapOption[] = POINTS.map((point) => ({
    key: point,
    tap: { point },
    label: t(POINT_LABEL[point]),
  }));
  const devices = Object.values(project.devices).filter((d) => d.track === track);
  const chainDevices = (pick: (d: Device) => boolean) => devices.filter(pick).sort(byOrder);
  const walk = (list: Device[], prefix: string, depth: number) => {
    if (depth > 8) return;
    for (const d of list) {
      const chains: {
        chain: SidechainChain;
        name: string;
        devices: Device[];
      }[] = [];
      for (const c of Object.values(project.rack_chains)
        .filter((c) => c.rack === d.id)
        .sort(byOrder)) {
        chains.push({
          chain: { type: "Rack", chain: c.id },
          name: c.name,
          devices: chainDevices((x) => x.chain === c.id),
        });
      }
      for (const pad of Object.values(project.drum_pads)
        .filter((p) => p.rack === d.id)
        .sort((a, b) => a.note - b.note)) {
        chains.push({
          chain: { type: "Pad", pad: pad.id },
          name: pad.name,
          devices: chainDevices((x) => x.pad === pad.id),
        });
      }
      for (const c of chains) {
        const path = `${prefix}${d.name} | ${c.name}`;
        for (const point of POINTS) {
          const tap: SidechainTap = { point, chain: c.chain };
          out.push({
            key: tapKey(tap),
            tap,
            label: `${path} | ${t(POINT_LABEL[point])}`,
          });
        }
        walk(c.devices, `${path} | `, depth + 1);
      }
    }
  };
  walk(
    chainDevices((d) => d.chain == null && d.pad === null),
    "",
    0,
  );
  return out;
}

/** The first valid tap of `source` for `device` (the post-fader output if allowed). */
export function defaultTap(project: P, device: Device, source: TrackId): SidechainTap | null {
  for (const tap of [POST_FADER, { point: "PreFx" } as SidechainTap]) {
    if (tapError(project, device, source, tap) === null) return tap;
  }
  return null;
}
