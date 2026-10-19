import type { Command, DeviceDescriptor, DeviceId, Project, Track } from "@/generated";
import { emit } from "@/ext";
import { useArrangementUi } from "@/features/arrangement/state";
import { canInsert, chainInsertBefore } from "@/features/devices/chainUtils";
import { builtinDevice } from "@/features/devices/descriptors";
import { addInstrumentCommand } from "@/features/devices/instrument";
import { resolveSelectedTrack } from "@/features/devices/selectedTrack";
import { devicesOfTrack, useSelectionStore } from "@/state";
import { cmd, newId, type EngineTransport } from "@/transport";

/**
 * Where "add device" goes: the arrangement's selected track (header), else the app's
 * selected track, else the first regular track.
 */
export function deviceTargetTrack(project: Project): Track | undefined {
  const id = useArrangementUi.getState().trackFocus ?? useSelectionStore.getState().selectedTrack;
  return resolveSelectedTrack(project, id);
}

/**
 * Adding a built-in device on `track`: an instrument replaces the track's instrument in place
 * (one undo step) or, with none, goes after the MIDI effects ([`addInstrumentCommand`]); a MIDI
 * effect goes before the instrument ([`chainInsertBefore`]); audio effects go last.
 */
export async function insertDeviceCommand(transport: EngineTransport, project: Project, track: Track, d: DeviceDescriptor): Promise<Command | null> {
  if (!canInsert(d, track) || d.device_type.type !== "Builtin") return null;
  const device = builtinDevice(d.device_type.device);
  emit("app.event", { type: "device_inserted", category: d.category, source: "builtin" });
  const devices = devicesOfTrack(project, track.id);
  const insert = (before: DeviceId | null): Command =>
    cmd("Device", { type: "Insert", id: newId(), track: track.id, device: { type: "Builtin", device }, before });
  return d.category === "Instrument" ? addInstrumentCommand(transport, devices, insert) : insert(chainInsertBefore(devices, d.category));
}
