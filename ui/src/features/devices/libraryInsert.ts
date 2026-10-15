// Adding a device picked in the device library to a track's chain.
import type { Command, DeviceCategory, DeviceId, DeviceSpec, Project, Track } from "@/generated";
import { chainInsertBefore } from "@/features/devices/chainUtils";
import { builtinDevice } from "./descriptors";
import { addInstrumentCommand } from "./instrument";
import { devicesOfTrack } from "@/state";
import { cmd, newId, type EngineTransport } from "@/transport";
import type { LibraryDevicePayload } from "./libraryDevice";
import { recordDeviceUse } from "./libraryStats";

/** Instruments and MIDI effects need notes: MIDI tracks only. A VCA takes no devices. */
export function canHost(category: DeviceCategory, track: Pick<Track, "kind">): boolean {
  if (track.kind === "Vca") return false;
  return category === "AudioEffect" || track.kind === "Midi";
}

export function deviceSpec(p: LibraryDevicePayload): DeviceSpec {
  return p.device.type === "Builtin"
    ? { type: "Builtin", device: builtinDevice(p.device.builtin) }
    : {
        type: "Plugin",
        plugin_id: p.device.plugin_id,
        format: p.device.format,
        sandboxed: null,
        // Which of the plugins sharing this format + id (VST2 uniqueID collisions).
        ...(p.device.shared && p.device.name !== undefined && { name: p.device.name }),
      };
}

/**
 * The command adding `p` to `track` (`null` when the track can't host it). An instrument
 * replaces the track's instrument (one undo step) or goes after the MIDI effects; effects go
 * at `before` when given (a position in their own section), else MIDI effects before the
 * instrument and audio effects last.
 */
export async function libraryInsertCommand(
  transport: EngineTransport,
  project: Project,
  track: Track,
  p: LibraryDevicePayload,
  before?: DeviceId | null,
): Promise<Command | null> {
  if (!canHost(p.category, track)) return null;
  recordDeviceUse(p);
  const devices = devicesOfTrack(project, track.id);
  const device = deviceSpec(p);
  const insert = (at: DeviceId | null): Command => cmd("Device", { type: "Insert", id: newId(), track: track.id, device, before: at });
  if (p.category === "Instrument") return addInstrumentCommand(transport, devices, insert);
  return insert(before !== undefined ? before : chainInsertBefore(devices, p.category));
}
