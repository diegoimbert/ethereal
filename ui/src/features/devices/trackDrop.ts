/**
 * Device cards dropped on a track in the arranger (its header or lane): the dragged card, or
 * the selection when it is part of it, goes to the end of each device's section on that track,
 * moved (copied with ⌘ / Ctrl held, like a clip drag), one undo step (`placeDevices`: an
 * instrument where the track has one replaces it once confirmed; what the track can't host
 * is skipped with a toast).
 */
import type { Device, Track, TrackId } from "@/generated";
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

/** Drops the dragged devices (`dt`) on `track`; `copy`: copies instead of moving. */
export async function dropDevicesOnTrack(transport: EngineTransport, dt: Pick<DataTransfer, "getData">, track: TrackId, copy: boolean): Promise<void> {
  const id = dt.getData(DEVICE_DRAG_TYPE);
  if (!id) return;
  const builtins = await fetchBuiltinTypes(transport).catch(() => []);
  const plugins = usePluginStore.getState().plugins;
  const categoryOf = (d: Device) => deviceCategory(d, builtins, plugins);
  await placeDevices(transport, draggedDevices(id), { type: "Track", track }, categoryOf, !copy);
}
