/**
 * Runtime plugin state that is not in the document: the scanned plugin list, scan
 * progress, crashed instances and open editor windows. Fed by `Event::Plugin` (see
 * `usePluginEvents`) and by the replies of plugin commands.
 *
 * While another computer's engine plays the project, `plugins` is **that computer's** list
 * (`setRemote`, set by whatever connects to it, e.g. an extension): everything that offers or
 * checks plugins reads it.
 */
import { useContext, useEffect } from "react";
import { create } from "zustand";
import type { DeviceId, Event, PluginDescriptor, PluginEvent, ScanFailure } from "@/generated";

/** The plugins of the computer whose engine plays the project, when it is not this one. */
export interface RemotePluginCatalog {
  /** That computer's name (shown: "plugins on <name>'s computer"). */
  name: string;
  plugins: PluginDescriptor[];
}
import { useProjectStore } from "@/state";
import { TransportContext, type EngineTransport } from "@/transport";
import { isInstalled } from "./filter";

export interface ScanState {
  done: number;
  total: number;
  current: string | null;
}

export interface PluginRuntimeState {
  /** The plugins to offer: the engine host's while `remote` is set, else `local`. */
  plugins: PluginDescriptor[] | null;
  /** This computer's scanned plugins (`Plugin::List`), `null` until loaded. */
  local: PluginDescriptor[] | null;
  /** The plugins of the computer whose engine plays the project, when not this one's. */
  remote: RemotePluginCatalog | null;
  /** Bumped per plugin device whose descriptor arrived from the engine host (fetch again). */
  descriptorEpochs: Record<DeviceId, number>;
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
  /** The engine host's list, or (`null`) back to this computer's. */
  setRemote(remote: RemotePluginCatalog | null): void;
  /** The engine host reported new descriptors for these plugin devices (fetch them again). */
  descriptorsChanged(devices: DeviceId[]): void;
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
  local: null,
  remote: null,
  descriptorEpochs: {},
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
  // `found` is about this computer's plugin host: only the local scan counts.
  setPlugins: (local) => set((s) => ({ local, plugins: s.remote ? s.remote.plugins : local, found: foundDevices(s.local, local, s.found) })),
  setRemote: (remote) => set((s) => ({ remote, plugins: remote ? remote.plugins : s.local })),
  descriptorsChanged: (devices) =>
    set((s) => {
      const descriptorEpochs = { ...s.descriptorEpochs };
      for (const d of devices) descriptorEpochs[d] = (descriptorEpochs[d] ?? 0) + 1;
      return { descriptorEpochs };
    }),
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
        // Forwarded editor keys are handled by `features/plugin-keys` listeners.
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
  if (e.type === "Plugin" && e.event.type === "ScanFinished" && usePluginStore.getState().local !== null) {
    refreshPluginList(transport);
  }
  // A new project has other devices: forget per-device runtime state.
  if (e.type === "ProjectLoaded") usePluginStore.setState({ crashed: {}, editors: {}, found: {}, descriptorEpochs: {} });
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
 * This computer's scanned plugin list (`null` until known), never the engine host's: what
 * is installed *here* (a plugin missing here opens its host's window, streamed; the local
 * reload / missing controls are about this plugin host). Fetches it once if nobody has yet
 * (desktop only; never when `enabled` is false), so device headers can tell a missing
 * plugin without the browser being open.
 */
export function useScannedPlugins(enabled: boolean): PluginDescriptor[] | null {
  const transport = useOptionalTransport();
  const local = usePluginStore((s) => s.local);
  useEffect(() => {
    if (enabled && local === null && transport) refreshPluginList(transport);
  }, [enabled, local, transport]);
  return local;
}

/** The transport, or `null` outside a `TransportProvider` (e.g. the bare app shell). */
export function useOptionalTransport(): EngineTransport | null {
  return useContext(TransportContext)?.transport ?? null;
}
