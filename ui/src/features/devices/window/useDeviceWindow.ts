import type { Device } from "@/generated";
import { setPluginWindow } from "@/features/plugins";
import { isInstalled } from "@/features/plugins/filter";
import { usePluginEvents, usePluginStore, useScannedPlugins } from "@/features/plugins/pluginStore";
import { useProjectStore } from "@/state";
import { cmd, useTransport, type EngineTransport } from "@/transport";
import { useDeviceWindows } from "./store";
import { isChainRack } from "@/features/racks/model";

export interface DeviceWindowControl {
  /** The device's window is open: the plugin's editor, or the factory device window. */
  open: boolean;
  /** It can be opened: not a crashed, missing or found-but-not-reloaded plugin, nor one held by safe mode. */
  available: boolean;
  toggle(): void;
  /** Opens it, or brings it to the front when it is already open (never closes it). */
  show(): void;
}

/**
 * Whether `device` has a window at all: not a chain rack (instrument / audio effect / MIDI
 * effect rack), whose macros and chains are on its card already.
 */
export function hasWindow(device: Pick<Device, "kind">): boolean {
  return !isChainRack(device as Device);
}

/**
 * The window of a device, whatever its kind (base-153): a plugin's native editor window
 * (`Plugin::OpenEditor` / `CloseEditor`), or a built-in device's floating window.
 */
export function useDeviceWindow(device: Device): DeviceWindowControl {
  const transport = useTransport();
  const plugin = device.kind.type === "Plugin" ? device.kind.plugin : null;
  // The plugin's window closing by itself (`EditorClosed`) or crashing reaches the store.
  usePluginEvents();
  const editorOpen = usePluginStore((s) => !!s.editors[device.id]);
  const crashed = usePluginStore((s) => s.crashed[device.id] !== undefined);
  const found = usePluginStore((s) => !!s.found[device.id]);
  const scanned = useScannedPlugins(plugin !== null);
  const held = useProjectStore((s) => s.safeMode.includes(device.id));
  const windowOpen = useDeviceWindows((s) => s.open.includes(device.id));
  const missing = plugin !== null && scanned !== null && !isInstalled(scanned, plugin);
  const open = plugin ? editorOpen : windowOpen;
  return {
    open,
    available: hasWindow(device) && (!plugin || (!crashed && !held && !missing && !found)),
    toggle: () => setDeviceWindow(transport, device, !open),
    show: () => showDeviceWindow(transport, device),
  };
}

/**
 * Opens a device's window, or brings it to the front when it is already open: a factory
 * window is raised; a plugin's editor gets `OpenEditor` again, which shows (raises) an open
 * editor on every plugin host.
 */
export function showDeviceWindow(transport: EngineTransport, device: Device): void {
  if (!hasWindow(device)) return;
  if (device.kind.type !== "Plugin") return useDeviceWindows.getState().setOpen(device.id, true);
  if (!usePluginStore.getState().editors[device.id]) return setPluginWindow(transport, device, true);
  if (usePluginStore.getState().crashed[device.id] !== undefined) return;
  transport.send(cmd("Plugin", { type: "OpenEditor", device: device.id })).catch((e: unknown) => console.warn("plugin window failed", e));
}

/** Opens or closes a device's window (plugin editor or factory window). */
export function setDeviceWindow(transport: EngineTransport, device: Device, open: boolean): void {
  if (open && !hasWindow(device)) return;
  if (device.kind.type === "Plugin") setPluginWindow(transport, device, open);
  else useDeviceWindows.getState().setOpen(device.id, open);
}
