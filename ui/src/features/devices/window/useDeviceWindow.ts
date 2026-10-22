import type { Device } from "@/generated";
import { contributions } from "@/ext";
import { setPluginWindow } from "@/features/plugins";
import { isMissing, usePluginEvents, usePluginStore, useScannedPlugins } from "@/features/plugins/pluginStore";
import { webclapOf } from "@/features/webclap/hooks";
import { useProjectStore } from "@/state";
import { cmd, useTransport, type EngineTransport } from "@/transport";
import { useDeviceWindows } from "./store";
import { useT } from "@/i18n";
import { isChainRack } from "@/features/racks/model";

export interface DeviceWindowControl {
  /** The device's window is open: the plugin's editor, or the factory device window. */
  open: boolean;
  /** It can be opened: not a crashed, missing or found-but-not-reloaded plugin, nor one held by safe mode. */
  available: boolean;
  toggle(): void;
  /** Opens it, or brings it to the front when it is already open (never closes it). */
  show(): void;
  /** The button's tooltip instead of the app's (an extension's window, `@/ext` "devices.window"). */
  title?: string;
  /** Unavailable, but `aria-disabled` rather than `disabled` (the tooltip still says why). */
  softDisabled?: boolean;
  /** Extra `data-*` attributes of the button (tests). */
  data?: Record<string, string>;
}

/** Whether this engine connection has a plugin host (the browser's engines don't). */
export function hasPluginHost(kind: EngineTransport["kind"]): boolean {
  return kind !== "wasm" && kind !== "mock";
}

/** A WebCLAP plugin device on the web version's engine: its window is a factory-like one. */
export function isWebclapWindow(transport: EngineTransport | null, device: Pick<Device, "kind">): boolean {
  return device.kind.type === "Plugin" && device.kind.plugin.format === "Webclap" && webclapOf(transport) !== null;
}

/**
 * Whether `device` has a window at all: not a chain rack (instrument / audio effect / MIDI
 * effect rack), whose macros and chains are on its card already.
 */
export function hasWindow(device: Pick<Device, "kind">): boolean {
  return !isChainRack(device as Device);
}

/**
 * The window of a device, whatever its kind: a plugin's native editor window
 * (`Plugin::OpenEditor` / `CloseEditor`), or a built-in device's floating window.
 */
export function useDeviceWindow(device: Device): DeviceWindowControl {
  const t = useT();
  const transport = useTransport();
  const plugin = device.kind.type === "Plugin" ? device.kind.plugin : null;
  // The plugin's window closing by itself (`EditorClosed`) or crashing reaches the store.
  usePluginEvents();
  const editorOpen = usePluginStore((s) => !!s.editors[device.id]);
  const crashed = usePluginStore((s) => s.crashed[device.id] !== undefined);
  const found = usePluginStore((s) => !!s.found[device.id]);
  const scanned = useScannedPlugins(plugin !== null);
  const covers = usePluginStore((s) => s.covers);
  const held = useProjectStore((s) => s.safeMode.includes(device.id));
  const windowOpen = useDeviceWindows((s) => s.open.includes(device.id));
  const missing = plugin !== null && isMissing(scanned, covers, plugin);
  // A window the app can't open itself (a plugin not installed here, or no plugin host at all):
  // an extension may (`@/ext` "devices.window"; hooks, called in order).
  // The web version hosts its WebCLAP plugins itself.
  const noPluginHost = !hasPluginHost(transport.kind) && !isWebclapWindow(transport, device);
  const missingHere = plugin !== null && (noPluginHost || missing);
  const ext = contributions("devices.window")
    // eslint-disable-next-line react-hooks/rules-of-hooks -- hooks, fixed once the app runs
    .map((useWindow) => useWindow(device, { missing: missingHere, noPluginHost }))
    .find((c) => c !== null);
  if (ext) return ext;
  // A WebCLAP plugin in the web version: the app's own window (its web ui, or its params).
  if (plugin && isWebclapWindow(transport, device)) {
    return {
      open: windowOpen,
      available: !crashed && !held && !missing && !found,
      toggle: () => useDeviceWindows.getState().setOpen(device.id, !windowOpen),
      show: () => useDeviceWindows.getState().setOpen(device.id, true),
    };
  }
  // No plugin host (the browser's engines): the editor can't open; the toggle says why.
  if (plugin && noPluginHost)
    return { open: false, available: false, softDisabled: true, title: t("devices.window.no_plugin_host"), toggle: () => {}, show: () => {} };
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
  if (device.kind.type !== "Plugin" || isWebclapWindow(transport, device)) return useDeviceWindows.getState().setOpen(device.id, true);
  if (!usePluginStore.getState().editors[device.id]) return setPluginWindow(transport, device, true);
  if (usePluginStore.getState().crashed[device.id] !== undefined) return;
  transport.send(cmd("Plugin", { type: "OpenEditor", device: device.id })).catch((e: unknown) => console.warn("plugin window failed", e));
}

/** Opens or closes a device's window (plugin editor or factory window). */
export function setDeviceWindow(transport: EngineTransport, device: Device, open: boolean): void {
  if (open && !hasWindow(device)) return;
  if (device.kind.type === "Plugin" && !isWebclapWindow(transport, device)) setPluginWindow(transport, device, open);
  else useDeviceWindows.getState().setOpen(device.id, open);
}
