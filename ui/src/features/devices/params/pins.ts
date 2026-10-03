import { useSyncExternalStore } from "react";
import type { Device, ParamId } from "@/generated";

/**
 * Params the user pinned to a device card, per plugin (format + plugin id; built-ins by
 * type), so every instance of that plugin in every project shows them. A UI setting kept
 * on this machine (localStorage `eth.devices.paramPins`), never in the project document.
 */
const KEY = "eth.devices.paramPins";
type Pins = Readonly<Record<string, ReadonlyArray<ParamId>>>;

const EMPTY: ReadonlyArray<ParamId> = [];
let pins: Pins = load();
const listeners = new Set<() => void>();

function load(): Pins {
  try {
    const v: unknown = JSON.parse(localStorage.getItem(KEY) ?? "{}");
    if (!v || typeof v !== "object" || Array.isArray(v)) return {};
    const out: Record<string, ParamId[]> = {};
    for (const [k, ids] of Object.entries(v as Record<string, unknown>)) {
      if (Array.isArray(ids)) out[k] = ids.filter((x): x is number => typeof x === "number");
    }
    return out;
  } catch {
    return {};
  }
}

function save(next: Pins): void {
  pins = next;
  try {
    localStorage.setItem(KEY, JSON.stringify(next));
  } catch {
    /* not persisted (storage unavailable) */
  }
  listeners.forEach((l) => l());
}

/** Pin key of a device: `<format>:<plugin id>` for plugins, `builtin:<type>` otherwise. */
export function pinKey(device: Pick<Device, "kind">): string {
  return device.kind.type === "Plugin"
    ? `${device.kind.plugin.format}:${device.kind.plugin.plugin_id}`
    : `builtin:${device.kind.device.type}`;
}

/** Pinned param ids of `key`, in pin order. */
export function getPins(key: string): ReadonlyArray<ParamId> {
  return pins[key] ?? EMPTY;
}

export function setPinned(key: string, param: ParamId, pinned: boolean): void {
  const cur = getPins(key);
  if (cur.includes(param) === pinned) return;
  const next = pinned ? [...cur, param] : cur.filter((p) => p !== param);
  const copy: Record<string, ReadonlyArray<ParamId>> = { ...pins };
  if (next.length) copy[key] = next;
  else delete copy[key];
  save(copy);
}

function subscribe(l: () => void): () => void {
  listeners.add(l);
  return () => listeners.delete(l);
}

/** Pinned param ids of `key` (re-renders when they change). */
export function usePins(key: string): ReadonlyArray<ParamId> {
  return useSyncExternalStore(subscribe, () => getPins(key));
}

/** Tests: forget every pin (and re-read storage). */
export function resetPins(): void {
  try {
    localStorage.removeItem(KEY);
  } catch {
    /* nothing saved */
  }
  pins = {};
  listeners.forEach((l) => l());
}

/** Tests: re-read storage, as after a reload. */
export function reloadPins(): void {
  pins = load();
  listeners.forEach((l) => l());
}
