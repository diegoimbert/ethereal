/**
 * Waveform peaks for audio clips, fetched from the engine with `Media::GetPeaks`.
 *
 * Peaks are requested in tiles of `TILE_PEAKS` peaks at power-of-two zoom levels
 * (`samples_per_peak`), so scrolling and zooming reuse what was already fetched. Tiles are
 * cached per media; `Media::PeaksReady` for a media drops its tiles (they are refetched).
 * Drawing code asks for the tiles it needs (`PeakCache.tile`), gets `null` for tiles still
 * loading, and redraws when `subscribe` fires.
 */

import type { MediaId, PeakData } from "@/generated";
import { cmd, type EngineTransport } from "@/transport";

export const TILE_PEAKS = 1024;
const MIN_LEVEL = 16;
const MAX_LEVEL = 1 << 20;

/** Power-of-two samples-per-peak level for `framesPerPx` source frames per pixel. */
export function peakLevel(framesPerPx: number): number {
  let level = MIN_LEVEL;
  while (level < framesPerPx && level < MAX_LEVEL) level *= 2;
  return level;
}

type Listener = () => void;

export class PeakCache {
  private tiles = new Map<string, PeakData | "loading" | "failed">();
  private listeners = new Set<Listener>();
  constructor(private transport: EngineTransport) {}

  /** A tile (`index` in units of `TILE_PEAKS * level` frames), or `null` while not loaded. */
  tile(media: MediaId, level: number, index: number): PeakData | null {
    const key = `${media}/${level}/${index}`;
    const hit = this.tiles.get(key);
    if (hit === undefined) {
      this.tiles.set(key, "loading");
      this.fetch(key, media, level, index);
      return null;
    }
    return typeof hit === "string" ? null : hit;
  }

  /** Forget every tile of `media` (e.g. `PeaksReady` after an import finished). */
  invalidate(media: MediaId): void {
    let any = false;
    for (const k of [...this.tiles.keys()]) {
      if (k.startsWith(`${media}/`)) {
        this.tiles.delete(k);
        any = true;
      }
    }
    if (any) this.notify();
  }

  subscribe(listener: Listener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  private notify() {
    for (const l of this.listeners) l();
  }

  private fetch(key: string, media: MediaId, level: number, index: number) {
    const request = { media, samples_per_peak: level, start_frame: index * TILE_PEAKS * level, frame_count: TILE_PEAKS * level };
    this.transport
      .send(cmd("Media", { type: "GetPeaks", request }))
      .then((reply) => {
        if (this.tiles.get(key) !== "loading") return; // invalidated meanwhile
        this.tiles.set(key, reply.type === "Peaks" ? reply.peaks : "failed");
        this.notify();
      })
      .catch(() => {
        if (this.tiles.get(key) === "loading") this.tiles.set(key, "failed");
      });
  }
}

/**
 * Min/max over source frames `[f0, f1)` across all channels, from whatever tiles are loaded
 * (`getTile(index)`). Returns `null` if no loaded peak covers the range.
 */
export function peakRange(
  f0: number,
  f1: number,
  level: number,
  getTile: (index: number) => PeakData | null,
): { min: number; max: number } | null {
  const tileFrames = TILE_PEAKS * level;
  let min = Infinity;
  let max = -Infinity;
  const end = Math.max(f1, f0 + 1);
  for (let ti = Math.floor(f0 / tileFrames); ti * tileFrames < end; ti++) {
    if (ti < 0) continue;
    const t = getTile(ti);
    if (!t) continue;
    const spp = Math.max(1, t.samples_per_peak);
    const n = t.max[0]?.length ?? 0;
    const i0 = Math.max(0, Math.floor((f0 - t.start_frame) / spp));
    const i1 = Math.min(n, Math.ceil((end - t.start_frame) / spp));
    for (let ch = 0; ch < t.max.length; ch++) {
      const mx = t.max[ch]!;
      const mn = t.min[ch]!;
      for (let i = i0; i < i1; i++) {
        if (mx[i]! > max) max = mx[i]!;
        if (mn[i]! < min) min = mn[i]!;
      }
    }
  }
  return max >= min ? { min, max } : null;
}
