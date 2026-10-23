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
  firstKey: number;
  lastKey: number;
  keyStep: number;
  velocitySteps: number;
  noteSeconds: number;
  tailSeconds: number;
}

/** Keys sampled for `[first, last]` by `step`, the last one included (like the engine). */
export function sampledKeyCount(first: number, last: number, step: number): number {
  if (last < first || step < 1) return 0;
  const hit = Math.floor((last - first) / step) + 1;
  return first + (hit - 1) * step === last ? hit : hit + 1;
}

export function sampleCount(o: Pick<ConvertOptions, "firstKey" | "lastKey" | "keyStep" | "velocitySteps">): number {
  return sampledKeyCount(o.firstKey, o.lastKey, o.keyStep) * Math.max(1, o.velocitySteps);
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
      first_key: o.firstKey,
      last_key: o.lastKey,
      key_step: o.keyStep,
      velocity_steps: o.velocitySteps,
      note_seconds: o.noteSeconds,
      tail_seconds: o.tailSeconds,
    }),
  );
}
