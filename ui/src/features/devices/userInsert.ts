/**
 * base-154: a device the user adds from a library or a menu (the chain's add buttons, the
 * command palette) opens its window: a plugin's editor, or a factory device's floating window
 * (before base-153 the new card expanded, which opened the plugin's editor). An insert that
 * can't happen or fails says so in a toast instead of doing nothing.
 *
 * Only for inserts the user asked for: not on load, undo / redo, duplicate, nor the agent's
 * inserts (it sends its commands itself).
 */
import type { Command, Device, DeviceCategory, DeviceId, Track } from "@/generated";
import { notify } from "@/features/notifications";
import { errorMessage } from "@/features/transport-bar/engine";
import { useProjectStore } from "@/state";
import type { EngineTransport } from "@/transport";
import type { LibraryDevicePayload } from "./libraryDevice";
import { libraryInsertCommand } from "./libraryInsert";
import { t } from "@/i18n";
import { setDeviceWindow } from "./window";

/** How long a new device may take to show up in the project (the engine's patch). */
const APPEAR_MS = 5000;

/** The id of the device `command` inserts (also inside a batch, e.g. "Replace Instrument"). */
export function insertedDeviceId(command: Command): DeviceId | null {
  if (command.domain === "Device" && command.command.type === "Insert") return command.command.id;
  if (command.domain === "Edit" && command.command.type === "Batch") {
    for (const c of command.command.commands) {
      const id = insertedDeviceId(c);
      if (id) return id;
    }
  }
  return null;
}

/** Why `track` can't take a device of `category` (`canHost` said no). */
export function cannotHostMessage(name: string, category: DeviceCategory, track: Pick<Track, "kind" | "name">): string {
  if (track.kind === "Vca") return t("devices.insert.vca", { track: track.name });
  return category === "Instrument" ? t("devices.insert.instrument_needs_midi", { name }) : t("devices.insert.midi_effect_needs_midi", { name });
}

/** The device `id` once it is in the project (null after `APPEAR_MS`). */
export function waitForDevice(id: DeviceId): Promise<Device | null> {
  const now = useProjectStore.getState().project?.devices[id];
  if (now) return Promise.resolve(now);
  return new Promise((resolve) => {
    const timer = setTimeout(() => {
      unsubscribe();
      resolve(null);
    }, APPEAR_MS);
    const unsubscribe = useProjectStore.subscribe((s) => {
      const d = s.project?.devices[id];
      if (!d) return;
      clearTimeout(timer);
      unsubscribe();
      resolve(d);
    });
  });
}

/**
 * Sends the insert of a device the user picked (`name`, for the error toast) and opens the new
 * device's window. Resolves to whether it was added.
 */
export async function sendUserInsert(transport: EngineTransport, command: Command, name: string): Promise<boolean> {
  try {
    await transport.send(command);
  } catch (e) {
    notify("Error", t("devices.insert.add_failed", { name, error: errorMessage(e) }));
    return false;
  }
  const id = insertedDeviceId(command);
  const device = id ? await waitForDevice(id) : null;
  if (device) setDeviceWindow(transport, device, true);
  return true;
}

/**
 * Adds a device picked in the device library to `track`: opens its window, or says why it
 * can't go there (e.g. an instrument on an audio track).
 */
export async function insertFromLibrary(transport: EngineTransport, track: Track, payload: LibraryDevicePayload): Promise<boolean> {
  const project = useProjectStore.getState().project;
  if (!project) return false;
  const command = await libraryInsertCommand(transport, project, track, payload);
  if (!command) {
    notify("Warning", cannotHostMessage(payload.name, payload.category, track));
    return false;
  }
  return sendUserInsert(transport, command, payload.name);
}
