/** Rack helpers: rack types and chain content rules (mirror `ether-controller` racks). */

import type { BuiltinDeviceType, Device, DeviceDescriptor, DeviceId, Project } from "@/generated";

const RACK_TYPES = new Set<BuiltinDeviceType>(["InstrumentRack", "AudioEffectRack", "MidiEffectRack"]);
/** Chain volume knob range (dB); the engine allows down to silence. */
export const VOL_MIN = -60;
export const VOL_MAX = 6;
/** Levels of racks inside racks (`ether_model::rack::MAX_RACK_DEPTH`). */
export const MAX_RACK_DEPTH = 6;

/** Whether `id` sits under a drum pad (a drum rack encloses it, at any depth). */
export function isUnderDrumPad(p: Pick<Project, "devices" | "rack_chains" | "drum_pads">, id: DeviceId): boolean {
  let d = p.devices[id];
  for (let i = 0; d && i <= MAX_RACK_DEPTH; i++) {
    if (d.pad != null) return true;
    const rack = d.chain != null ? p.rack_chains[d.chain]?.rack : undefined;
    if (rack === undefined) return false;
    d = p.devices[rack];
  }
  return false;
}

export function isChainRack(d: Device): boolean {
  return d.kind.type === "Builtin" && RACK_TYPES.has(d.kind.device.type);
}

export function rackTypeOf(d: Device): BuiltinDeviceType | null {
  return d.kind.type === "Builtin" && RACK_TYPES.has(d.kind.device.type) ? d.kind.device.type : null;
}

/**
 * The nesting level of rack `id`: 1 on a track chain, 2 on a chain of a level-1 rack (or on a
 * pad of a track-chain drum rack), ... Every rack on the way down counts, drum racks included.
 */
export function rackLevel(p: Pick<Project, "devices" | "rack_chains"> & Partial<Pick<Project, "drum_pads">>, id: DeviceId): number {
  let level = 1;
  let d = p.devices[id];
  while (d && level <= MAX_RACK_DEPTH) {
    const rack = d.chain != null ? p.rack_chains[d.chain]?.rack : d.pad != null ? p.drum_pads?.[d.pad]?.rack : undefined;
    if (rack === undefined) break;
    level += 1;
    d = p.devices[rack];
  }
  return level;
}

/**
 * Can a device go on a chain of a `rack` rack? Audio effect racks take audio effects, MIDI
 * effect racks MIDI effects, instrument racks anything, a Drum Rack included (as the chain's
 * instrument) unless the rack sits under a drum pad (`underDrumPad`: no drum rack ever does);
 * racks nest (of a kind that fits) unless `nestRacks` is false (the depth limit).
 */
export function fitsChain(rack: BuiltinDeviceType, d: DeviceDescriptor, nestRacks = true, underDrumPad = false): boolean {
  if (d.device_type.type === "Builtin") {
    const ty = d.device_type.device;
    if (ty === "DrumRack" && (rack !== "InstrumentRack" || underDrumPad || !nestRacks)) return false;
    if (RACK_TYPES.has(ty) && !nestRacks) return false;
  }
  if (rack === "AudioEffectRack") return d.category === "AudioEffect";
  if (rack === "MidiEffectRack") return d.category === "NoteEffect";
  return true;
}
