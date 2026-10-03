/** Rack helpers: rack types and chain content rules (mirror `ether-controller` racks). */

import type { BuiltinDeviceType, Device, DeviceDescriptor } from "@/generated";

const RACK_TYPES = new Set<BuiltinDeviceType>(["InstrumentRack", "AudioEffectRack", "MidiEffectRack"]);
/** Chain volume knob range (dB); the engine allows down to silence. */
export const VOL_MIN = -60;
export const VOL_MAX = 6;

export function isChainRack(d: Device): boolean {
  return d.kind.type === "Builtin" && RACK_TYPES.has(d.kind.device.type);
}

export function rackTypeOf(d: Device): BuiltinDeviceType | null {
  return d.kind.type === "Builtin" && RACK_TYPES.has(d.kind.device.type) ? d.kind.device.type : null;
}

/** Can a device of `category` go on a chain of a `rack` rack? (no nesting either) */
export function fitsChain(rack: BuiltinDeviceType, d: DeviceDescriptor): boolean {
  if (d.device_type.type !== "Builtin") return true;
  const ty = d.device_type.device;
  if (RACK_TYPES.has(ty) || ty === "DrumRack") return false;
  if (rack === "AudioEffectRack") return d.category === "AudioEffect";
  if (rack === "MidiEffectRack") return d.category === "NoteEffect";
  return true;
}

