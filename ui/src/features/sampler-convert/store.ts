/**
 * "Convert to sampler" (the instrument rack's context menu): opens a dialog; confirming
 * renders every chosen key/velocity through the instrument and replaces it with a
 * multisampler (`FreezeCommand::ConvertToSampler`).
 */

import { create } from "zustand";
import type { Device, DeviceCategory } from "@/generated";
import type { ContextMenuEntry } from "@/kit";
import { frozenAncestor, jobOfTrack, runRenderJob, useFreezeUi } from "@/features/freeze";
import { useProjectStore } from "@/state";
import { cmd, newId, type EngineTransport } from "@/transport";
import { t } from "@/i18n";

/** `ether_model::multisampler::MAX_ZONES`: the most samples one conversion may produce. */
export const MAX_CONVERT_SAMPLES = 512;

interface ConvertState {
  /** The device whose dialog is open. */
  device: Device | null;
  open(device: Device): void;
  close(): void;
}

export const useSamplerConvert = create<ConvertState>()((set) => ({
  device: null,
  open: (device) => set({ device }),
  close: () => set({ device: null }),
}));

/** The menu entry on an instrument on a track's own chain (not inside a rack or pad). */
export function convertToSamplerEntries(device: Device, categoryOf: (d: Device) => DeviceCategory): ContextMenuEntry[] {
  const p = useProjectStore.getState().project;
  const track = p?.tracks[device.track];
  if (!p || !track || track.kind !== "Midi") return [];
  if (device.pad != null || device.chain != null) return [];
  if (categoryOf(device) !== "Instrument") return [];
  if (track.freeze || frozenAncestor(p.tracks, track.id)) return [];
  if (jobOfTrack(useFreezeUi.getState(), track.id)) return [];
  return [{ label: t("sampler.convert.menu"), onSelect: () => useSamplerConvert.getState().open(device) }];
}

export interface ConvertOptions {
  /** Sampled keys (0..=127). */
  keys: number[];
  /** Rendered velocities (1..=127). */
  velocities: number[];
  noteSeconds: number;
  tailSeconds: number;
}

/** `sample::MAX_VELOCITIES`: the most velocities one conversion renders. */
export const MAX_CONVERT_VELOCITIES = 16;

/** Semitones between the keys of each key preset (12: every octave). */
export const KEY_PRESET_STEPS = [1, 2, 3, 4, 6, 12] as const;
/** Velocity counts of the velocity presets. */
export const VELOCITY_PRESET_COUNTS = [1, 2, 3, 4, 5, 8, 16] as const;
/** Keys a key preset spans when fewer than two keys are picked (C1–C6). */
export const DEFAULT_KEY_SPAN: readonly [number, number] = [36, 96];

/** Sorted, duplicates dropped. */
export function sortedSet(values: Iterable<number>): number[] {
  return [...new Set(values)].sort((a, b) => a - b);
}

/**
 * The keys of the "every `step` semitones" preset: from the lowest to the highest of `keys`
 * (`DEFAULT_KEY_SPAN` with fewer than two), the highest included when the step misses it.
 */
export function presetKeys(step: number, keys: readonly number[] = []): number[] {
  const sorted = sortedSet(keys);
  const [lo, hi] = sorted.length >= 2 ? [sorted[0]!, sorted.at(-1)!] : DEFAULT_KEY_SPAN;
  const out: number[] = [];
  for (let k = lo; k <= hi; k += Math.max(1, step)) out.push(k);
  if (out.at(-1) !== hi) out.push(hi);
  return out;
}

/** The velocities of the `count`-velocity preset: evenly up to 127 (127 always rendered). */
export function presetVelocities(count: number): number[] {
  const n = Math.max(1, Math.round(count));
  return Array.from({ length: n }, (_, i) => Math.max(1, Math.round((127 * (i + 1)) / n)));
}

/** A velocity to add to `velocities`: the middle of the widest gap in 1..=127 (null if none is left). */
export function velocityToAdd(velocities: readonly number[]): number | null {
  const v = sortedSet(velocities);
  let best: number | null = null;
  let width = 0;
  const edges = [0, ...v, 128];
  for (let i = 0; i + 1 < edges.length; i++) {
    const [a, b] = [edges[i]!, edges[i + 1]!];
    // Inside the gap (a, b), the edges 0 and 128 not being velocities.
    const mid = b === 128 ? 127 : a === 0 ? Math.max(1, Math.floor(b / 2)) : Math.floor((a + b) / 2);
    if (b - a > width && mid > a && mid < b && !v.includes(mid)) [best, width] = [mid, b - a];
  }
  return best;
}

export function sampleCount(o: Pick<ConvertOptions, "keys" | "velocities">): number {
  return sortedSet(o.keys).length * sortedSet(o.velocities).length;
}

export function sampleNoteName(key: number): string {
  const names = ["c", "c#", "d", "d#", "e", "f", "f#", "g", "g#", "a", "a#", "b"] as const;
  return `${names[((key % 12) + 12) % 12]}${Math.floor(key / 12) - 2}`;
}

/** Render and replace `device`; the job shows in its track's header and is queued like freeze. */
export function convertToSampler(transport: EngineTransport, device: Device, o: ConvertOptions): Promise<void> {
  return runRenderJob(transport, "Sample", [device.track], (job) =>
    cmd("Freeze", {
      type: "ConvertToSampler",
      job,
      device: device.id,
      new_device: newId(),
      seed_media: newId(),
      keys: sortedSet(o.keys),
      velocities: sortedSet(o.velocities),
      note_seconds: o.noteSeconds,
      tail_seconds: o.tailSeconds,
    }),
  );
}
