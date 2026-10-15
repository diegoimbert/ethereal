import { useSyncExternalStore } from "react";
import type { DeviceId } from "@/generated";

/**
 * Which device cards show their inline parameter list (the header's "Parameters" toggle,
 * base-153); every other card shows its header only. Independent of the device's window.
 * In memory only: the inspector opens a track with every list closed (`collapseAll`);
 * opening one keeps it open while the track stays shown.
 *
 * Migration (base-153): this used to be the card's expanded state (which also opened a
 * plugin's window). It was never saved, so nothing maps over: every card starts with its
 * parameters hidden and its window closed.
 */
let expanded: ReadonlySet<DeviceId> = new Set();
const listeners = new Set<() => void>();

export function setParamsOpen(id: DeviceId, open: boolean): void {
  if (expanded.has(id) === open) return;
  const next = new Set(expanded);
  if (open) next.add(id);
  else next.delete(id);
  expanded = next;
  listeners.forEach((l) => l());
}

export function useParamsOpen(id: DeviceId): boolean {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => expanded.has(id),
  );
}

/** Hides every card's parameter list (the inspector showing another track, or opening). */
export function collapseAll(): void {
  if (expanded.size === 0) return;
  expanded = new Set();
  listeners.forEach((l) => l());
}

/** Tests: back to the initial state (every parameter list hidden). */
export const resetCollapsed = collapseAll;
