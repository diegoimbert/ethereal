/** "Group into rack" (`Rack::Group`) for a device's context menu. */

import type { BuiltinDeviceType, Command, Device, DeviceDescriptor } from "@/generated";
import { t, type MessageKey } from "@/i18n";
import type { ContextMenuEntry } from "@/kit";
import { cmd, newId } from "@/transport";

/** The rack type that can hold a device of this category. */
export function rackTypeFor(category: DeviceDescriptor["category"]): BuiltinDeviceType {
  return category === "Instrument" ? "InstrumentRack" : category === "NoteEffect" ? "MidiEffectRack" : "AudioEffectRack";
}

const GROUP_INTO: Partial<Record<BuiltinDeviceType, MessageKey>> = {
  InstrumentRack: "racks.group.instrument",
  AudioEffectRack: "racks.group.audio_effect",
  MidiEffectRack: "racks.group.midi_effect",
};

/** Menu entries wrapping `device` into a new rack (track-chain devices; racks too, not drum racks). */
export function groupEntries(device: Device, descriptor: DeviceDescriptor | null, send: (c: Command) => unknown): ContextMenuEntry[] {
  if (!descriptor || device.pad !== null || device.chain != null) return [];
  if (device.kind.type === "Builtin" && device.kind.device.type === "DrumRack") return [];
  const type = rackTypeFor(descriptor.category);
  const label = GROUP_INTO[type];
  if (!label) return [];
  return [
    {
      label: t(label),
      onSelect: () => void send(cmd("Rack", { type: "Group", rack: newId(), rack_type: type, chain: newId(), devices: [device.id] })),
    },
  ];
}
