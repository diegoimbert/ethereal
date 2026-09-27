/**
 * Runtime plugin state that is not in the document: the scanned plugin list, scan
 * progress, crashed instances and open editor windows. Fed by `Event::Plugin` (see
 * `usePluginEvents`) and by the replies of plugin commands.
 */
import { useContext, useEffect } from "react";
import { create } from "zustand";
import type { DeviceId, Event, PluginDescriptor, PluginEvent, ScanFailure } from "@/generated";
import { TransportContext, type EngineTransport } from "@/transport";

export interface ScanState {
  done: number;
  total: number;
  current: string | null;
}

export interface PluginRuntimeState {
  /** Scanned plugins (`Plugin::List`), `null` until loaded. */
  plugins: PluginDescriptor[] | null;
  /** Rescan in progress. */
  scan: ScanState | null;
  /** Failures of the last finished scan. */
  failed: ScanFailure[];
  /** Crashed instances (bypassed until reloaded), with the crash message. */
  crashed: Record<DeviceId, string>;
  /** Devices whose editor window is open. */
  editors: Record<DeviceId, true>;
  setPlugins(plugins: PluginDescriptor[]): void;
  setScan(scan: ScanState | null): void;
  setEditorOpen(device: DeviceId, open: boolean): void;
  clearCrash(device: DeviceId): void;
  apply(event: PluginEvent): void;
  reset(): void;
}

const initial = {
  plugins: null,
  scan: null,
  failed: [],
  crashed: {},
  editors: {},
} satisfies Partial<PluginRuntimeState>;

function without<T>(map: Record<DeviceId, T>, device: DeviceId): Record<DeviceId, T> {
  if (!(device in map)) return map;
  const next = { ...map };
  delete next[device];
  return next;
}

export const usePluginStore = create<PluginRuntimeState>((set) => ({
  ...initial,
  setPlugins: (plugins) => set({ plugins }),
  setScan: (scan) => set({ scan }),
  setEditorOpen: (device, open) =>
    set((s) => ({ editors: open ? { ...s.editors, [device]: true } : without(s.editors, device) })),
  clearCrash: (device) => set((s) => ({ crashed: without(s.crashed, device) })),
  apply: (e) =>
    set((s) => {
      switch (e.type) {
        case "ScanProgress":
          return { scan: { done: e.done, total: e.total, current: e.current } };
        case "ScanFinished":
          return { scan: null, failed: e.failed };
        case "EditorClosed":
          return { editors: without(s.editors, e.device) };
        case "Crashed":
          return { crashed: { ...s.crashed, [e.device]: e.message }, editors: without(s.editors, e.device) };
        case "LatencyChanged":
          return {};
      }
    }),
  reset: () => set(initial),
}));

/** Per-transport subscription shared by every mounted plugin component. */
const subscriptions = new WeakMap<EngineTransport, { count: number; unsubscribe: () => void }>();

function onEvent(e: Event, transport: EngineTransport) {
  if (e.type === "Plugin") usePluginStore.getState().apply(e.event);
  // Device headers show missing plugins against the list: keep it current after a rescan.
  if (e.type === "Plugin" && e.event.type === "ScanFinished" && usePluginStore.getState().plugins !== null) {
    refreshPluginList(transport);
  }
  // A new project has other devices: forget per-device runtime state.
  if (e.type === "ProjectLoaded") usePluginStore.setState({ crashed: {}, editors: {} });
}

/**
 * Keep the plugin store fed with `Event::Plugin` while the calling component is mounted
 * (one transport subscription however many components call it).
 */
export function usePluginEvents(): void {
  const transport = useOptionalTransport();
  useEffect(() => {
    if (!transport) return;
    let sub = subscriptions.get(transport);
    if (!sub) {
      sub = { count: 0, unsubscribe: transport.onEvent((e) => onEvent(e, transport)) };
      subscriptions.set(transport, sub);
    }
    sub.count += 1;
    const current = sub;
    return () => {
      current.count -= 1;
      if (current.count === 0) {
        current.unsubscribe();
        subscriptions.delete(transport);
      }
    };
  }, [transport]);
}

/** In-flight `Plugin::List` per transport. */
const listing = new WeakMap<EngineTransport, Promise<void>>();

/** Fetch the scanned plugin list into the store (desktop only; one request at a time). */
function refreshPluginList(transport: EngineTransport): void {
  if (transport.kind !== "tauri" || listing.has(transport)) return;
  const pending = transport
    .send({ domain: "Plugin", command: { type: "List" } })
    .then((reply) => {
      if (reply.type === "Plugins") usePluginStore.getState().setPlugins(reply.plugins);
    })
    .catch(() => {})
    .finally(() => listing.delete(transport));
  listing.set(transport, pending);
}

/**
 * The scanned plugin list (`null` until known). Fetches it once if nobody has yet (desktop
 * only; never when `enabled` is false), so device headers can tell a missing plugin
 * without the browser being open.
 */
export function useScannedPlugins(enabled: boolean): PluginDescriptor[] | null {
  const transport = useOptionalTransport();
  const plugins = usePluginStore((s) => s.plugins);
  useEffect(() => {
    if (enabled && plugins === null && transport) refreshPluginList(transport);
  }, [enabled, plugins, transport]);
  return plugins;
}

/** The transport, or `null` outside a `TransportProvider` (e.g. the bare app shell). */
export function useOptionalTransport(): EngineTransport | null {
  return useContext(TransportContext)?.transport ?? null;
}
