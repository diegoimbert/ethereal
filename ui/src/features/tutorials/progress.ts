// Which tours the user finished. There is no engine-side store for UI preferences yet (the
// keymap has its own command), so this lives in localStorage like the other per-device UI
// settings; every read and write is guarded (private windows, blocked storage).
import { create } from "zustand";
import type { TourDef } from "./types";

const KEY = "eth.tutorials.done";

/** `${id}@${version}`: a rewritten tour (new version) is not done any more. */
export const doneKey = (tour: Pick<TourDef, "id" | "version">): string => `${tour.id}@${tour.version}`;

function read(): Record<string, number> {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "{}") as unknown;
    if (!v || typeof v !== "object" || Array.isArray(v)) return {};
    return Object.fromEntries(Object.entries(v).filter((e): e is [string, number] => typeof e[1] === "number"));
  } catch {
    return {};
  }
}

function write(done: Record<string, number>): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(done));
  } catch {
    // Storage unavailable: remembered for this session only.
  }
}

interface ProgressState {
  /** `doneKey` → when it was finished (ms). */
  done: Record<string, number>;
}

export const useTutorialProgress = create<ProgressState>()(() => ({ done: read() }));

export function isTourDone(tour: Pick<TourDef, "id" | "version">): boolean {
  return doneKey(tour) in useTutorialProgress.getState().done;
}

export function markTourDone(tour: Pick<TourDef, "id" | "version">, now: number = Date.now()): void {
  const done = { ...useTutorialProgress.getState().done, [doneKey(tour)]: now };
  useTutorialProgress.setState({ done });
  write(done);
}

/** Tests: forget everything (and re-read the storage). */
export function resetTutorialProgress(clearStorage = false): void {
  if (clearStorage) {
    try {
      localStorage.removeItem(KEY);
    } catch {
      /* unavailable */
    }
  }
  useTutorialProgress.setState({ done: read() });
}
