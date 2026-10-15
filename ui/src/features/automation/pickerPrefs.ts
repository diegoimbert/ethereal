import { useSyncExternalStore } from "react";

/**
 * "Active only" filter of the automation parameter pickers: a UI preference shared by every
 * track and clip picker, kept on this machine (localStorage `eth.automation.pickerActiveOnly`),
 * never in the project document.
 */
const KEY = "eth.automation.pickerActiveOnly";

function load(): boolean {
  try {
    return localStorage.getItem(KEY) === "1";
  } catch {
    return false;
  }
}

let activeOnly = load();
const listeners = new Set<() => void>();

export function setPickerActiveOnly(on: boolean): void {
  activeOnly = on;
  try {
    localStorage.setItem(KEY, on ? "1" : "0");
  } catch {
    /* not persisted (storage unavailable) */
  }
  listeners.forEach((l) => l());
}

/** Re-read storage (as after a page reload; for tests). */
export function reloadPickerPrefs(): void {
  activeOnly = load();
  listeners.forEach((l) => l());
}

/** The stored preference and its setter. */
export function usePickerActiveOnly(): readonly [boolean, (on: boolean) => void] {
  const v = useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => activeOnly,
  );
  return [v, setPickerActiveOnly];
}
