/** "Group into rack" (`Rack::Group`) for a device's context menu. */

import type { BuiltinDeviceType, Command, Device, DeviceDescriptor } from "@/generated";
import type { ContextMenuEntry } from "@/kit";
import { cmd, newId } from "@/transport";

/** The rack type that can hold a device of this category. */
export function rackTypeFor(category: DeviceDescriptor["category"]): BuiltinDeviceType {
  return category === "Instrument" ? "InstrumentRack" : category === "NoteEffect" ? "MidiEffectRack" : "AudioEffectRack";
}

const NAMES: Partial<Record<BuiltinDeviceType, string>> = {
  InstrumentRack: "Instrument Rack",
  AudioEffectRack: "Audio Effect Rack",
  MidiEffectRack: "MIDI Effect Rack",
};

/** Menu entries wrapping `device` into a new rack (track-chain devices, not racks). */
export function groupEntries(device: Device, descriptor: DeviceDescriptor | null, send: (c: Command) => unknown): ContextMenuEntry[] {
  if (!descriptor || device.pad !== null || device.chain != null) return [];
  if (device.kind.type === "Builtin" && (NAMES[device.kind.device.type] || device.kind.device.type === "DrumRack")) return [];
  const type = rackTypeFor(descriptor.category);
  return [
    {
      label: `Group into ${NAMES[type]}`,
      onSelect: () => void send(cmd("Rack", { type: "Group", rack: newId(), rack_type: type, chain: newId(), devices: [device.id] })),
    },
  ];
}
