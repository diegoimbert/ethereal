/**
 * Presets: pure helpers and the per-session "current preset" of each device.
 *
 * The engine owns the files (factory presets embedded, user presets in the user library);
 * the UI only lists, loads and saves through `Preset::*` commands (CONTRACTS.md §12.5).
 */

import { create } from "zustand";
import type { Device, DeviceId, PresetDevice, PresetInfo, PresetRef } from "@/generated";

/** The preset type of a device (built-in type, or plugin identity). */
export function presetDeviceOf(device: Device): PresetDevice {
  if (device.kind.type === "Builtin") return { type: "Builtin", device: device.kind.device.type };
  const { format, plugin_id, name, vendor } = device.kind.plugin;
  return { type: "Plugin", format, plugin_id, name, vendor };
}

export const sameRef = (a: PresetRef, b: PresetRef) => a.source === b.source && a.id === b.id;

/** A user preset of the same list with this name (case-insensitive), except `except`. */
export function findByName(presets: ReadonlyArray<PresetInfo>, name: string, except?: PresetRef): PresetInfo | undefined {
  const n = name.trim().toLowerCase();
  if (!n) return undefined;
  return presets.find((p) => p.preset.source === "User" && p.name.toLowerCase() === n && !(except && sameRef(p.preset, except)));
}

/** "pad, Warm , pad" → ["pad", "warm"] (the engine normalizes the same way). */
export function parseTags(text: string): string[] {
  const tags = text
    .split(",")
    .map((t) => t.trim().toLowerCase())
    .filter((t) => t.length > 0);
  return [...new Set(tags)].sort();
}

/** Factory presets first, then user presets (the engine already sorts each by name). */
export function groupPresets(presets: ReadonlyArray<PresetInfo>): {
  factory: PresetInfo[];
  user: PresetInfo[];
} {
  return {
    factory: presets.filter((p) => p.preset.source === "Factory"),
    user: presets.filter((p) => p.preset.source === "User"),
  };
}

interface CurrentPresets {
  /** The preset last loaded or saved on each device in this session (shown in its header). */
  current: Record<DeviceId, { ref: PresetRef; name: string }>;
  set(device: DeviceId, ref: PresetRef, name: string): void;
  /** Forget/rename a preset that was deleted or renamed. */
  replace(ref: PresetRef, next: { ref: PresetRef; name: string } | null): void;
}

export const useCurrentPresets = create<CurrentPresets>((set) => ({
  current: {},
  set: (device, ref, name) => set((s) => ({ current: { ...s.current, [device]: { ref, name } } })),
  replace: (ref, next) =>
    set((s) => {
      const current = { ...s.current };
      for (const [d, c] of Object.entries(current)) {
        if (!sameRef(c.ref, ref)) continue;
        if (next) current[d] = next;
        else delete current[d];
      }
      return { current };
    }),
}));
