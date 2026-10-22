/**
 * Runtime plugin state that is not in the document: the scanned plugin list, scan
 * progress, crashed instances and open editor windows. Fed by `Event::Plugin` (see
 * `usePluginEvents`) and by the replies of plugin commands.
 *
 * While another computer's engine plays the project, `plugins` is **that computer's** list
 * (`setRemote`, set by whatever connects to it, e.g. an extension): everything that offers or
 * checks plugins reads it. Whether a plugin is missing is the engine host's answer too
 * (`engineCatalog`): a plugin only missing on this computer isn't.
 */
import { useContext, useEffect } from "react";
import { create } from "zustand";
import type { DeviceId, Event, PluginDescriptor, PluginEvent, PluginFormat, ScanFailure } from "@/generated";

/** The plugins of the computer whose engine plays the project, when it is not this one. */
export interface RemotePluginCatalog {
  /** That computer's name (shown: "plugins on <name>'s computer"). */
  name: string;
  plugins: PluginDescriptor[];
}
import { useProjectStore } from "@/state";
import { TransportContext, type EngineTransport } from "@/transport";
import { webclapOf } from "@/features/webclap/hooks";
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
  /**
   * The formats `local` speaks for (`null`: all of them). The web version's list holds its
   * WebCLAP plugins only: a native plugin there is not "missing" (it can't be installed in
   * a browser at all; its device stays bypassed as before).
   */
  covers: PluginFormat[] | null;
  /** The plugins of the computer whose engine plays the project, when not this one's. */
  remote: RemotePluginCatalog | null;
  /** Another computer's engine plays the project, but its list hasn't come yet (or never
   * comes: a view-only listener gets none): which plugins are missing is unknown meanwhile. */
  remotePending: boolean;
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
  /** Plugin devices still to load (instantiated a few per engine tick when a project opens). */
  loading: DeviceId[];
  /** Plugin devices whose plugin was missing from the list and a later list has it: the
   * host couldn't load it, so it is still bypassed until reloaded ("found · reload"). */
  found: Record<DeviceId, true>;
  /** Set the scanned list; plugin devices of the open project it newly has go to `found`. */
  setPlugins(plugins: PluginDescriptor[], covers?: PluginFormat[] | null): void;
  /** The engine host's list, or (`null`) back to this computer's. */
  setRemote(remote: RemotePluginCatalog | null): void;
  /** Another computer's engine plays the project (`remote` may still be on its way). */
  setRemotePending(pending: boolean): void;
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
  covers: null,
  remote: null,
  remotePending: false,
  descriptorEpochs: {},
  scan: null,
  failed: [],
  retrying: {},
  crashed: {},
  editors: {},
  found: {},
  loading: [],
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
  setPlugins: (local, covers = null) =>
    set((s) => ({ local, covers, plugins: s.remote ? s.remote.plugins : local, found: foundDevices(s.local, local, s.found) })),
  setRemote: (remote) => set((s) => ({ remote, plugins: remote ? remote.plugins : s.local })),
  setRemotePending: (remotePending) => set((s) => (s.remotePending === remotePending ? s : { remotePending })),
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
        case "Loading":
          return { loading: e.pending };
        // Forwarded editor keys are handled by `features/plugin-keys` listeners.
        case "LatencyChanged":
        case "EditorKey":
          return {};
      }
    }),
  reset: () => set(initial),
}));

/**
 * The plugins of the computer whose engine plays the project, which say what is missing:
 * the engine host's list while another computer's engine plays it (`null` until it comes),
 * else this computer's scan (`null` until loaded).
 */
export function engineCatalog(s: Pick<PluginRuntimeState, "local" | "remote" | "remotePending">): PluginDescriptor[] | null {
  if (s.remote) return s.remote.plugins;
  return s.remotePending ? null : s.local;
}

/**
 * Whether a device's plugin is missing from `plugins` (the engine host's list, `null`:
 * unknown), for the formats that list covers (`covers`, `null`: every format).
 */
export function isMissing(
  plugins: readonly PluginDescriptor[] | null,
  covers: readonly PluginFormat[] | null,
  plugin: { format: PluginFormat; plugin_id: string },
): boolean {
  if (plugins === null) return false;
  if (covers !== null && !covers.includes(plugin.format)) return false;
  return !isInstalled(plugins, plugin);
}

/** `covers` of the list `engineCatalog` returns (another computer's list covers every format). */
export function engineCovers(s: Pick<PluginRuntimeState, "covers" | "remote">): PluginFormat[] | null {
  return s.remote ? null : s.covers;
}

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
  if (e.type === "ProjectLoaded") usePluginStore.setState({ crashed: {}, editors: {}, found: {}, descriptorEpochs: {}, loading: [] });
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
      const off = transport.onEvent((e) => onEvent(e, transport));
      // The web version's WebCLAP plugins: installs and removals change the list.
      const offWebclap = webclapOf(transport)?.subscribe(() => refreshPluginList(transport, true));
      sub = {
        count: 0,
        unsubscribe: () => {
          off();
          offWebclap?.();
        },
      };
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

/**
 * Fetch the scanned plugin list into the store (one request at a time): the desktop's scan,
 * or the web version's installed WebCLAP plugins. `again`: once the request in flight ends
 * (the list changed meanwhile).
 */
export function refreshPluginList(transport: EngineTransport, again = false): void {
  const web = webclapOf(transport) !== null;
  if (transport.kind !== "tauri" && !web) return;
  const inFlight = listing.get(transport);
  if (inFlight) {
    if (again) void inFlight.then(() => refreshPluginList(transport));
    return;
  }
  const pending = transport
    .send({ domain: "Plugin", command: { type: "List" } })
    .then((reply) => {
      if (reply.type === "Plugins") usePluginStore.getState().setPlugins(reply.plugins, web ? ["Webclap"] : null);
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

/**
 * The plugins installed where the project's engine runs (`engineCatalog`), `null` while
 * unknown: what device headers check a plugin device against for "missing". Fetches this
 * computer's scan once if nobody has yet, like `useScannedPlugins`.
 */
export function useEnginePlugins(enabled: boolean): PluginDescriptor[] | null {
  useScannedPlugins(enabled);
  return usePluginStore(engineCatalog);
}

/** The transport, or `null` outside a `TransportProvider` (e.g. the bare app shell). */
export function useOptionalTransport(): EngineTransport | null {
  return useContext(TransportContext)?.transport ?? null;
}
