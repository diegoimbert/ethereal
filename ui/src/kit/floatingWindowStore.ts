/**
 * The floating windows' shared state (kit `FloatingWindow`).
 *
 * - **Geometry** per window id (position, and size once resized): a UI setting kept on this
 *   machine (localStorage `eth.windows`), never in a project. The most recently used
 *   `MAX_RECTS` are kept.
 * - **Stacking order** of the windows on screen (bottom to top): not persisted; a window
 *   rises when it opens and when it's clicked.
 */
import { create } from "zustand";
import type { WindowRect } from "./floatingWindowGeometry";

const KEY = "eth.windows";
const MAX_RECTS = 200;

export interface FloatingWindowsState {
  rects: Readonly<Record<string, WindowRect>>;
  /** Windows on screen, bottom to top. */
  order: ReadonlyArray<string>;
  setRect(id: string, rect: WindowRect): void;
  /** Brings `id` to the top (adds it when it isn't on screen yet). */
  raise(id: string): void;
  /** `id` left the screen. */
  remove(id: string): void;
}

const finite = (v: unknown): v is number => typeof v === "number" && Number.isFinite(v);

function load(): Record<string, WindowRect> {
  const out: Record<string, WindowRect> = {};
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "{}") as Record<string, Partial<WindowRect>> | null;
    if (!v || typeof v !== "object") return out;
    for (const [id, r] of Object.entries(v)) {
      if (!r || !finite(r.x) || !finite(r.y)) continue;
      const rect: WindowRect = { x: r.x, y: r.y };
      if (finite(r.w) && r.w > 0) rect.w = r.w;
      if (finite(r.h) && r.h > 0) rect.h = r.h;
      out[id] = rect;
    }
  } catch {
    /* nothing remembered (storage unavailable or corrupt) */
  }
  return out;
}

function save(rects: Readonly<Record<string, WindowRect>>): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(rects));
  } catch {
    /* not persisted (storage unavailable) */
  }
}

export const useFloatingWindows = create<FloatingWindowsState>((set, get) => ({
  rects: load(),
  order: [],
  setRect: (id, rect) =>
    set((s) => {
      const next: Record<string, WindowRect> = { ...s.rects };
      delete next[id];
      next[id] = rect;
      const ids = Object.keys(next);
      for (const old of ids.slice(0, Math.max(0, ids.length - MAX_RECTS))) delete next[old];
      return { rects: next };
    }),
  raise: (id) => {
    const { order } = get();
    if (order.at(-1) === id) return;
    set({ order: [...order.filter((w) => w !== id), id] });
  },
  remove: (id) => {
    const { order } = get();
    if (order.includes(id)) set({ order: order.filter((w) => w !== id) });
  },
}));

useFloatingWindows.subscribe((s, prev) => {
  if (s.rects !== prev.rects) save(s.rects);
});

/** How each window on screen closes (its `onClose`), by id. */
const closers = new Map<string, () => void>();

/** `FloatingWindow`: register (or, with `null`, forget) how window `id` closes. */
export function setWindowCloser(id: string, close: (() => void) | null): void {
  if (close) closers.set(id, close);
  else closers.delete(id);
}

/**
 * Closes the topmost window on screen (desktop ⌘W in the main window). Returns whether one
 * was closed.
 */
export function closeTopFloatingWindow(): boolean {
  const { order } = useFloatingWindows.getState();
  for (const id of [...order].reverse()) {
    const close = closers.get(id);
    if (close) {
      close();
      return true;
    }
  }
  return false;
}

/** Remembered geometry of a window (open or not), e.g. to migrate older settings. */
export function windowRect(id: string): WindowRect | undefined {
  return useFloatingWindows.getState().rects[id];
}

/** Tests: no remembered geometry, nothing on screen (storage cleared too). */
export function resetFloatingWindows(): void {
  useFloatingWindows.setState({ rects: {}, order: [] });
  try {
    localStorage.removeItem(KEY);
  } catch {
    /* ignore */
  }
}
