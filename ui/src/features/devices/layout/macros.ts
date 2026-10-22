/**
 * A rack's macro bank settings (`ether_model::RackMacros` on its `BuiltinDevice`, every rack
 * type, drum racks included): how many macros it shows and their custom names, and the
 * "rename in progress" state the `Macros` widget and the param menu ("rename macro") share.
 */

import { create } from "zustand";
import type { Device, DeviceId, RackMacros } from "@/generated";

/** Macros per rack (`ether_model::RACK_MACROS`). */
export const RACK_MACROS = 16;
/** Macros a rack shows by default (`ether_model::DEFAULT_VISIBLE_MACROS`). */
export const DEFAULT_VISIBLE_MACROS = 8;

const DEFAULTS: RackMacros = { visible: DEFAULT_VISIBLE_MACROS, names: [] };

/** `d`'s macro bank settings (the defaults when unset, or for a device without macros). */
export function rackMacros(d: Pick<Device, "kind"> | undefined | null): RackMacros {
  const k = d?.kind;
  if (!k || k.type !== "Builtin" || !("macros" in k.device)) return DEFAULTS;
  return k.device.macros ?? DEFAULTS;
}

/** How many macros `d` shows (`RackMacros::visible`, `1..=RACK_MACROS`). */
export function visibleMacros(d: Pick<Device, "kind"> | undefined | null): number {
  return Math.min(RACK_MACROS, Math.max(1, rackMacros(d).visible));
}

/** `d`'s custom name of macro `index`, or `null` (the default "macro N"). */
export function customMacroName(d: Pick<Device, "kind"> | undefined | null, index: number): string | null {
  const n = rackMacros(d).names[index]?.trim();
  return n ? n : null;
}

interface RenameState {
  /** The macro being renamed in place, or `null`. */
  editing: { device: DeviceId; index: number } | null;
  start(device: DeviceId, index: number): void;
  stop(): void;
}

/** The macro whose name is being edited (the `Macros` widget shows the field). */
export const useMacroRename = create<RenameState>()((set) => ({
  editing: null,
  start: (device, index) => set({ editing: { device, index } }),
  stop: () => set({ editing: null }),
}));
