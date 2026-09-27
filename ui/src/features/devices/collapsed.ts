import { useSyncExternalStore } from "react";
import type { DeviceId } from "@/generated";

/**
 * Which device cards are collapsed in the stacked (inspector) layout, per device,
 * remembered across sessions (localStorage; in memory only where storage is unavailable).
 */
const KEY = "eth.devices.collapsed";
let collapsed: ReadonlySet<DeviceId> = load();
const listeners = new Set<() => void>();

function load(): ReadonlySet<DeviceId> {
  try {
    const v: unknown = JSON.parse(localStorage.getItem(KEY) ?? "[]");
    return new Set(Array.isArray(v) ? (v as DeviceId[]) : []);
  } catch {
    return new Set();
  }
}

export function setCollapsed(id: DeviceId, value: boolean): void {
  if (collapsed.has(id) === value) return;
  const next = new Set(collapsed);
  if (value) next.add(id);
  else next.delete(id);
  collapsed = next;
  try {
    localStorage.setItem(KEY, JSON.stringify([...next]));
  } catch {
    /* not persisted */
  }
  listeners.forEach((l) => l());
}

export function useCollapsed(id: DeviceId): boolean {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => collapsed.has(id),
  );
}

/** Tests: expand everything. */
export function resetCollapsed(): void {
  collapsed = new Set();
  try {
    localStorage.removeItem(KEY);
  } catch {
    /* nothing saved */
  }
  listeners.forEach((l) => l());
}
