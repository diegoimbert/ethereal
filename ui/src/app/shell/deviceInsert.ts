import type { Command, DeviceDescriptor, DeviceId, Project, Track } from "@/generated";
import { useArrangementUi } from "@/features/arrangement/state";
import { builtinDevice } from "@/features/devices/descriptors";
import { addInstrumentCommand } from "@/features/devices/instrument";
import { resolveSelectedTrack } from "@/features/devices/selectedTrack";
import { devicesOfTrack, useSelectionStore } from "@/state";
import { cmd, newId, type EngineTransport } from "@/transport";

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

/**
 * Adding a built-in device on `track`: effects go last; an instrument replaces the track's
 * instrument in place (one undo step), or goes first when it has none.
 */
export async function insertDeviceCommand(
  transport: EngineTransport,
  project: Project,
  track: Track,
  d: DeviceDescriptor,
): Promise<Command | null> {
  if (!canInsert(d, track) || d.device_type.type !== "Builtin") return null;
  const device = builtinDevice(d.device_type.device);
  const insert = (before: DeviceId | null): Command =>
    cmd("Device", { type: "Insert", id: newId(), track: track.id, device: { type: "Builtin", device }, before });
  return d.category === "Instrument" ? addInstrumentCommand(transport, devicesOfTrack(project, track.id), insert) : insert(null);
}
