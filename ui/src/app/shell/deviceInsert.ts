import type { Command, DeviceDescriptor, Project, Track } from "@/generated";
import { useArrangementUi } from "@/features/arrangement/state";
import { builtinDevice } from "@/features/devices/descriptors";
import { resolveSelectedTrack } from "@/features/devices/selectedTrack";
import { devicesOfTrack, useSelectionStore } from "@/state";
import { cmd, newId } from "@/transport";

/** Category headings, in display order. */
export const DEVICE_CATEGORIES: ReadonlyArray<{ id: DeviceDescriptor["category"]; label: string }> = [
  { id: "Instrument", label: "Instruments" },
  { id: "AudioEffect", label: "Audio effects" },
  { id: "NoteEffect", label: "MIDI effects" },
];

/**
 * Where "add device" goes: the arrangement's selected track (header), else the app's
 * selected track, else the first regular track.
 */
export function deviceTargetTrack(project: Project): Track | undefined {
  const id = useArrangementUi.getState().trackFocus ?? useSelectionStore.getState().selectedTrack;
  return resolveSelectedTrack(project, id);
}

/** Instruments only go on MIDI tracks. */
export function canInsert(d: DeviceDescriptor, track: Track | undefined): boolean {
  return !!track && d.device_type.type === "Builtin" && (d.category !== "Instrument" || track.kind === "Midi");
}

/** `Device::Insert` of a built-in device on `track`: instruments first in the chain, effects last. */
export function insertDeviceCommand(project: Project, track: Track, d: DeviceDescriptor): Command | null {
  if (!canInsert(d, track) || d.device_type.type !== "Builtin") return null;
  const first = devicesOfTrack(project, track.id)[0]?.id ?? null;
  return cmd("Device", {
    type: "Insert",
    id: newId(),
    track: track.id,
    device: { type: "Builtin", device: builtinDevice(d.device_type.device) },
    before: d.category === "Instrument" ? first : null,
  });
}
