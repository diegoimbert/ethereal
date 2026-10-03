import type { BuiltinDeviceType, Command, Device, DeviceDescriptor, DeviceId, Project, Track } from "@/generated";
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

/** Built-in MIDI effects (category `NoteEffect`). Plugins aren't constrained by the chain rule. */
const NOTE_EFFECTS: ReadonlySet<BuiltinDeviceType> = new Set<BuiltinDeviceType>([
  "Arpeggiator",
  "Chord",
  "ScaleQuantize",
  "NoteLength",
  "Velocity",
  "Randomizer",
  "MidiEffectRack",
]);

const isNoteEffect = (d: Device): boolean => d.kind.type === "Builtin" && NOTE_EFFECTS.has(d.kind.device.type);

/**
 * Where a new device of `category` goes in a chain (`before`; `null` = append), per the chain
 * rule (CONTRACTS.md §12.4.4: MIDI effects precede the instrument). A MIDI effect, or an
 * instrument on a track without one, goes before the first device that isn't a MIDI effect;
 * audio effects append.
 */
export function chainInsertBefore(devices: readonly Device[], category: DeviceDescriptor["category"]): DeviceId | null {
  if (category === "AudioEffect") return null;
  return devices.find((d) => !isNoteEffect(d))?.id ?? null;
}

/**
 * Adding a built-in device on `track`: an instrument replaces the track's instrument in place
 * (one undo step) or, with none, goes after the MIDI effects ([`addInstrumentCommand`]); a MIDI
 * effect goes before the instrument ([`chainInsertBefore`]); audio effects go last.
 */
export async function insertDeviceCommand(
  transport: EngineTransport,
  project: Project,
  track: Track,
  d: DeviceDescriptor,
): Promise<Command | null> {
  if (!canInsert(d, track) || d.device_type.type !== "Builtin") return null;
  const device = builtinDevice(d.device_type.device);
  const devices = devicesOfTrack(project, track.id);
  const insert = (before: DeviceId | null): Command =>
    cmd("Device", { type: "Insert", id: newId(), track: track.id, device: { type: "Builtin", device }, before });
  return d.category === "Instrument" ? addInstrumentCommand(transport, devices, insert) : insert(chainInsertBefore(devices, d.category));
}
