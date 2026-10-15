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

type TileState = PeakData | "loading" | "failed";

export class PeakCache {
  /** media → level → tile index → tile. Nested maps with number keys: drawing asks for a
   *  tile per pixel column, and a string key per call was a hot spot while zooming. */
  private tiles = new Map<MediaId, Map<number, Map<number, TileState>>>();
  private listeners = new Set<Listener>();
  private pending = false;
  constructor(private transport: EngineTransport) {}

  /** A tile (`index` in units of `TILE_PEAKS * level` frames), or `null` while not loaded. */
  tile(media: MediaId, level: number, index: number): PeakData | null {
    let levels = this.tiles.get(media);
    if (!levels) this.tiles.set(media, (levels = new Map()));
    let tiles = levels.get(level);
    if (!tiles) levels.set(level, (tiles = new Map()));
    const hit = tiles.get(index);
    if (hit === undefined) {
      tiles.set(index, "loading");
      this.fetch(tiles, media, level, index);
      return null;
    }
    return typeof hit === "string" ? null : hit;
  }

  /** A tile if it is loaded, without requesting it (fallbacks while another level loads). */
  peek(media: MediaId, level: number, index: number): PeakData | null {
    const hit = this.tiles.get(media)?.get(level)?.get(index);
    return hit === undefined || typeof hit === "string" ? null : hit;
  }

  /** Forget every tile of `media` (e.g. `PeaksReady` after an import finished). */
  invalidate(media: MediaId): void {
    if (this.tiles.delete(media)) this.notify();
  }

  subscribe(listener: Listener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  private notify() {
    for (const l of this.listeners) l();
  }

  /** `notify` once per frame for tile arrivals: a zoom requests tiles for every lane at
   *  once, and a redraw of every lane per reply was a storm of re-renders. */
  private notifySoon() {
    if (this.pending) return;
    this.pending = true;
    const run = () => {
      this.pending = false;
      this.notify();
    };
    if (typeof requestAnimationFrame === "function") requestAnimationFrame(run);
    else setTimeout(run, 0);
  }

  private fetch(tiles: Map<number, TileState>, media: MediaId, level: number, index: number) {
    const request = { media, samples_per_peak: level, start_frame: index * TILE_PEAKS * level, frame_count: TILE_PEAKS * level };
    // Invalidated meanwhile: `tiles` is no longer this media's map (or the tile was reset).
    const current = () => this.tiles.get(media)?.get(level) === tiles && tiles.get(index) === "loading";
    this.transport
      .send(cmd("Media", { type: "GetPeaks", request }))
      .then((reply) => {
        if (!current()) return;
        tiles.set(index, reply.type === "Peaks" ? reply.peaks : "failed");
        this.notifySoon();
      })
      .catch(() => {
        if (current()) tiles.set(index, "failed");
      });
  }
}

/**
 * Min/max over source frames `[f0, f1)` across all channels, from whatever tiles are loaded
 * (`getTile(index)`). Returns `null` if no loaded peak covers the range.
 */
export function peakRange(f0: number, f1: number, level: number, getTile: (index: number) => PeakData | null): { min: number; max: number } | null {
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
