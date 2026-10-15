/**
 * Deterministic fake waveform peaks for `Media::GetPeaks` in the mock: a drum-loop-like
 * shape (decaying hits every half second, accents every second) plus hashed noise, so the
 * same request always returns the same data and zooming looks coherent.
 */

import type { MediaRef, PeakData, PeakRequest } from "@/generated";

/** Hash of (seed, index) → [0, 1). */
function hash01(seed: number, i: number): number {
  let h = (seed ^ Math.imul(i, 0x9e3779b1)) >>> 0;
  h = Math.imul(h ^ (h >>> 16), 0x85ebca6b);
  h = Math.imul(h ^ (h >>> 13), 0xc2b2ae35);
  return ((h ^ (h >>> 16)) >>> 0) / 4294967296;
}

function stringSeed(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619);
  return h >>> 0;
}

/** Max peaks per reply (hosts would stream/tile larger requests). */
const MAX_PEAKS = 1 << 16;

export function synthesizePeaks(media: MediaRef, req: PeakRequest): PeakData {
  const spp = Math.max(1, Math.floor(req.samples_per_peak));
  const start = Math.max(0, Math.floor(req.start_frame));
  const end = Math.min(media.frames, start + Math.max(0, req.frame_count));
  const count = Math.min(MAX_PEAKS, Math.max(0, Math.ceil((end - start) / spp)));
  const seed = stringSeed(media.id);
  const min: number[][] = [];
  const max: number[][] = [];
  for (let ch = 0; ch < media.channels; ch++) {
    const mins = new Array<number>(count);
    const maxs = new Array<number>(count);
    for (let i = 0; i < count; i++) {
      const frame = start + i * spp;
      const t = frame / media.sample_rate;
      const inHit = t % 0.5;
      const accent = Math.floor(t / 0.5) % 2 === 0 ? 1 : 0.65;
      const env = Math.exp(-inHit * 9) * accent;
      // Noise is keyed by the absolute frame block, so it's stable across requests.
      const block = Math.floor(frame / 256);
      const noise = hash01(seed + ch * 7919, block);
      const amp = Math.min(1, 0.04 + env * (0.75 + 0.2 * noise) + 0.05 * noise);
      maxs[i] = Math.round(amp * 1000) / 1000;
      mins[i] = -Math.round(amp * (0.85 + 0.15 * hash01(seed, block + 1)) * 1000) / 1000;
    }
    min.push(mins);
    max.push(maxs);
  }
  return { media: media.id, samples_per_peak: spp, start_frame: start, min, max };
}
