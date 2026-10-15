/**
 * Runtime plugin state that is not in the document: the scanned plugin list, scan
 * progress, crashed instances and open editor windows. Fed by `Event::Plugin` (see
 * `usePluginEvents`) and by the replies of plugin commands.
 */
import { useContext, useEffect } from "react";
import { create } from "zustand";
import type { DeviceId, Event, PluginDescriptor, PluginEvent, ScanFailure } from "@/generated";
import { useProjectStore } from "@/state";
import { TransportContext, type EngineTransport } from "@/transport";
import { isInstalled } from "./filter";

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
  /** Targets that failed to load: the last scan's (or, before one, the engine's
   * `ListScanFailures`), updated by retries. */
  failed: ScanFailure[];
  /** Failures being retried (`RetryScan` in flight), by `failureKey`. */
  retrying: Record<string, true>;
  /** Crashed instances (bypassed until reloaded), with the crash message. */
  crashed: Record<DeviceId, string>;
  /** Devices whose editor window is open. */
  editors: Record<DeviceId, true>;
  /** Plugin devices whose plugin was missing from the list and a later list has it: the
   * host couldn't load it, so it is still bypassed until reloaded ("found · reload"). */
  found: Record<DeviceId, true>;
  /** Set the scanned list; plugin devices of the open project it newly has go to `found`. */
  setPlugins(plugins: PluginDescriptor[]): void;
  setScan(scan: ScanState | null): void;
  setFailed(failed: ScanFailure[]): void;
  setRetrying(failure: ScanFailure, busy: boolean): void;
  /** A retry ended: drop the row (`failure` null: it loaded) or show its new reason. */
  retried(target: ScanFailure, failure: ScanFailure | null): void;
  setEditorOpen(device: DeviceId, open: boolean): void;
  clearCrash(device: DeviceId): void;
  /** The device was reloaded (or is gone): no longer `found`. */
  clearFound(device: DeviceId): void;
  apply(event: PluginEvent): void;
  reset(): void;
}

const initial = {
  plugins: null,
  scan: null,
  failed: [],
  retrying: {},
  crashed: {},
  editors: {},
  found: {},
} satisfies Partial<PluginRuntimeState>;

/** Identifies a failed target (a path may be scanned as several formats). */
export function failureKey(f: ScanFailure): string {
  return `${f.format ?? ""}:${f.path}`;
}

function without<T>(map: Record<DeviceId, T>, device: DeviceId): Record<DeviceId, T> {
  if (!(device in map)) return map;
  const next = { ...map };
  delete next[device];
  return next;
}

export const usePluginStore = create<PluginRuntimeState>((set) => ({
  ...initial,
  setPlugins: (plugins) => set((s) => ({ plugins, found: foundDevices(s.plugins, plugins, s.found) })),
  setScan: (scan) => set({ scan }),
  setFailed: (failed) => set({ failed }),
  setRetrying: (f, busy) =>
    set((s) => {
      const key = failureKey(f);
      if (busy) return { retrying: { ...s.retrying, [key]: true } };
      const next = { ...s.retrying };
      delete next[key];
      return { retrying: next };
    }),
  retried: (target, failure) =>
    set((s) => {
      const key = failureKey(target);
      return {
        failed: failure ? s.failed.map((f) => (failureKey(f) === key ? { ...f, message: failure.message } : f)) : s.failed.filter((f) => failureKey(f) !== key),
      };
    }),
  setEditorOpen: (device, open) => set((s) => ({ editors: open ? { ...s.editors, [device]: true } : without(s.editors, device) })),
  clearCrash: (device) => set((s) => ({ crashed: without(s.crashed, device) })),
  clearFound: (device) => set((s) => ({ found: without(s.found, device) })),
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
        // transport-keys: forwarded editor keys are handled by `features/plugin-keys` listeners.
        case "LatencyChanged":
        case "EditorKey":
          return {};
      }
    }),
  reset: () => set(initial),
}));

/**
 * `found` after the list goes from `prev` to `next`: the open project's plugin devices
 * missing from `prev` and listed in `next` join it (the host refused to create them, so
 * they stay bypassed until reloaded). Unknown before the first list (`prev` null).
 */
function foundDevices(prev: PluginDescriptor[] | null, next: PluginDescriptor[], found: Record<DeviceId, true>): Record<DeviceId, true> {
  const devices = useProjectStore.getState().project?.devices;
  if (prev === null || !devices) return found;
  let out = found;
  for (const d of Object.values(devices)) {
    if (d.kind.type !== "Plugin" || d.id in out) continue;
    if (!isInstalled(prev, d.kind.plugin) && isInstalled(next, d.kind.plugin)) out = { ...out, [d.id]: true };
  }
  return out;
}

/** Per-transport subscription shared by every mounted plugin component. */
const subscriptions = new WeakMap<EngineTransport, { count: number; unsubscribe: () => void }>();

function onEvent(e: Event, transport: EngineTransport) {
  if (e.type === "Plugin") usePluginStore.getState().apply(e.event);
  // Device headers show missing plugins against the list: keep it current after a rescan.
  if (e.type === "Plugin" && e.event.type === "ScanFinished" && usePluginStore.getState().plugins !== null) {
    refreshPluginList(transport);
  }
  // A new project has other devices: forget per-device runtime state.
  if (e.type === "ProjectLoaded") usePluginStore.setState({ crashed: {}, editors: {}, found: {} });
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
