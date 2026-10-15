/**
 * base-153: factory (built-in) device windows. A built-in device's editor (its declared
 * layout, sample slot) opens in a floating window, like a plugin's editor window; the device
 * card in the chain only shows its header (and, on demand, the generic parameter list).
 *
 * Which windows are open (in stacking order, the last one on top) is a UI setting kept on this
 * machine (localStorage `eth.devices.windows`), per device id, never in the project document:
 * like plugin editor windows, opening one is not an edit (no undo step, not shared in collab).
 * Where each window sits (and its size) is the kit `FloatingWindow`'s geometry under the window
 * id `deviceWindowId(device)` (base-154; positions saved here before are moved there once).
 * Device ids are unique across projects, so a project opened again brings its windows back
 * where they were.
 */
import { create } from "zustand";
import type { DeviceId } from "@/generated";
import { resetFloatingWindows, useFloatingWindows } from "@/kit";

const KEY = "eth.devices.windows";

/** The kit `FloatingWindow` id of a device's window (its remembered geometry). */
export function deviceWindowId(device: DeviceId): string {
  return `device:${device}`;
}

export interface DeviceWindowsState {
  /** Open windows, bottom to top. */
  open: ReadonlyArray<DeviceId>;
  setOpen(device: DeviceId, open: boolean): void;
  toggle(device: DeviceId): void;
  /** Brings an open window to the top. */
  raise(device: DeviceId): void;
  /** Closes the windows of devices that were removed. */
  closeMany(devices: ReadonlyArray<DeviceId>): void;
}

const finite = (v: unknown): v is number => typeof v === "number" && Number.isFinite(v);

function load(): DeviceId[] {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "{}") as { open?: unknown; positions?: Record<string, { x?: unknown; y?: unknown } | null> } | null;
    const open = Array.isArray(v?.open) ? v.open.filter((x): x is string => typeof x === "string") : [];
    // Positions remembered before base-154 move to the kit's window geometry, once.
    if (v?.positions && typeof v.positions === "object") {
      const kit = useFloatingWindows.getState();
      for (const [id, p] of Object.entries(v.positions)) {
        const wid = deviceWindowId(id);
        if (p && finite(p.x) && finite(p.y) && !kit.rects[wid]) kit.setRect(wid, { x: p.x, y: p.y });
      }
      save(open);
    }
    return open;
  } catch {
    return [];
  }
}

function save(open: ReadonlyArray<DeviceId>): void {
  try {
    localStorage.setItem(KEY, JSON.stringify({ open }));
  } catch {
    /* not persisted (storage unavailable) */
  }
}

export const useDeviceWindows = create<DeviceWindowsState>((set, get) => ({
  open: load(),
  setOpen: (device, open) => {
    const s = get();
    const without = s.open.filter((d) => d !== device);
    // Opening an open window raises it.
    set({ open: open ? [...without, device] : without });
    if (open) useFloatingWindows.getState().raise(deviceWindowId(device));
  },
  toggle: (device) => get().setOpen(device, !get().open.includes(device)),
  raise: (device) => {
    const { open } = get();
    if (open.at(-1) === device || !open.includes(device)) return;
    set({ open: [...open.filter((d) => d !== device), device] });
  },
  closeMany: (devices) => {
    const gone = new Set(devices);
    const { open } = get();
    if (open.some((d) => gone.has(d))) set({ open: open.filter((d) => !gone.has(d)) });
  },
}));

useDeviceWindows.subscribe((s, prev) => {
  if (s.open !== prev.open) save(s.open);
});

/** Tests: no window open, no remembered geometry (storage cleared too). */
export function resetDeviceWindows(): void {
  useDeviceWindows.setState({ open: [] });
  resetFloatingWindows();
  try {
    localStorage.removeItem(KEY);
  } catch {
    /* ignore */
  }
}
