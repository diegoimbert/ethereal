/**
 * Device cards dropped on a track in the arranger (its header or lane): the dragged card, or
 * the selection when it is part of it, goes to the end of each device's section on that track,
 * moved (copied with ⌘ / Ctrl held, like a clip drag), one undo step (`placeDevices`: an
 * instrument where the track has one replaces it once confirmed; what the track can't host
 * is skipped with a toast).
 */
import type { Device, DeviceCategory, DeviceId, Project, Track, TrackId } from "@/generated";
import { usePluginStore } from "@/features/plugins/pluginStore";
import type { EngineTransport } from "@/transport";
import { DEVICE_DRAG_TYPE, deviceCategory } from "./chainUtils";
import { fetchBuiltinTypes } from "./descriptors";
import { placeDevices, useDeviceClipboard } from "./deviceClipboard";

/** `true` while a device card is dragged (usable during `dragover`). */
export const isDeviceDrag = (types: ReadonlyArray<string>) => types.includes(DEVICE_DRAG_TYPE);

/** Whether `track` can take dropped devices (a VCA takes none). */
export const acceptsDevices = (track: Track): boolean => track.kind !== "Vca";

/** The devices a drag of card `id` carries: the selection when the card is part of it. */
export function draggedDevices(id: string): string[] {
  const { selected } = useDeviceClipboard.getState();
  return selected.includes(id) ? [...selected] : [id];
}

/**
 * What a drag of card `id` moves within the device chains: the card, or the selection when
 * it is part of it, keeping the devices of the card's kind (a move stays in its section),
 * leaving out those inside a moved rack (they go with it), in chain order.
 */
export function movedDevices(project: Project, id: DeviceId, categoryOf: (d: Device) => DeviceCategory): DeviceId[] {
  const grabbed = project.devices[id];
  if (!grabbed) return [id];
  const kind = categoryOf(grabbed);
  const devices = draggedDevices(id)
    .map((d) => project.devices[d])
    .filter((d): d is Device => !!d && categoryOf(d) === kind);
  const ids = new Set(devices.map((d) => d.id));
  const inMoved = (d: Device): boolean => {
    const rack = d.chain != null ? project.rack_chains[d.chain]?.rack : d.pad !== null ? project.drum_pads[d.pad]?.rack : undefined;
    return rack !== undefined && (ids.has(rack) || (!!project.devices[rack] && inMoved(project.devices[rack])));
  };
  return devices
    .filter((d) => !inMoved(d))
    .sort((a, b) => (a.order < b.order ? -1 : a.order > b.order ? 1 : a.id < b.id ? -1 : 1))
    .map((d) => d.id);
}

/** Drops the dragged devices (`dt`) on `track`; `copy`: copies instead of moving. */
export async function dropDevicesOnTrack(transport: EngineTransport, dt: Pick<DataTransfer, "getData">, track: TrackId, copy: boolean): Promise<void> {
  const id = dt.getData(DEVICE_DRAG_TYPE);
  if (!id) return;
  const builtins = await fetchBuiltinTypes(transport).catch(() => []);
  const plugins = usePluginStore.getState().plugins;
  const categoryOf = (d: Device) => deviceCategory(d, builtins, plugins);
  await placeDevices(transport, draggedDevices(id), { type: "Track", track }, categoryOf, !copy);
}
