/**
 * Waveform peaks on the GPU: the peak tiles of `PeakCache` (1024 min/max peaks per tile,
 * power-of-two levels) as rows of one `RG32F` texture (min, max over the channels). Rows are
 * assigned on demand and recycled least-recently-used; a tile keeps its row while it is
 * drawn. The CPU copy of each row stays, so a lost context re-uploads everything.
 */

import type { PeakData } from "@/generated";
import type { GL, GlContext } from "./glContext";
import { TILE_PEAKS } from "../peaks";

/** Rows of the tile texture (8 KiB each). */
export const TILE_ROWS = 512;

interface Slot {
  tile: PeakData | null;
  data: Float32Array;
  used: number;
}

export class WaveTiles {
  private slots: Slot[] = [];
  private byTile = new WeakMap<PeakData, number>();
  private pending = new Set<number>();
  private texture: WebGLTexture | null = null;
  private generation = -1;
  /** Frame counter (rows used in the current frame are never recycled). */
  private frame = 0;
  /** Bumped when a row is recycled: instances built before may point at another tile now. */
  version = 0;

  /** GPU bytes of the texture. */
  get bytes(): number {
    return this.texture ? TILE_PEAKS * TILE_ROWS * 8 : 0;
  }

  /** Rows in use. */
  get used(): number {
    return this.slots.filter((s) => s.tile).length;
  }

  /** Next frame of row use (call once per drawn frame, before the lanes build their view
   *  data: a row any lane uses in the frame is not recycled by a later lane). */
  tick(): void {
    this.frame++;
  }

  /** Marks rows as drawn this frame (they are not recycled while on screen). */
  touch(rows: ReadonlyArray<number>): void {
    for (const r of rows) {
      const s = this.slots[r];
      if (s) s.used = this.frame;
    }
  }

  /** The texture row holding `tile` (assigned and queued for upload if new), or -1 if full. */
  row(tile: PeakData): number {
    const known = this.byTile.get(tile);
    if (known !== undefined && this.slots[known]?.tile === tile) {
      this.slots[known]!.used = this.frame;
      return known;
    }
    let at = -1;
    if (this.slots.length < TILE_ROWS) {
      at = this.slots.length;
      this.slots.push({ tile: null, data: new Float32Array(TILE_PEAKS * 2), used: 0 });
    } else {
      let oldest = Infinity;
      this.slots.forEach((s, i) => {
        if (s.used < oldest && s.used !== this.frame) {
          oldest = s.used;
          at = i;
        }
      });
      if (at < 0) return -1;
      this.version++;
    }
    const slot = this.slots[at]!;
    slot.tile = tile;
    slot.used = this.frame;
    fillRow(slot.data, tile);
    this.byTile.set(tile, at);
    this.pending.add(at);
    return at;
  }

  /** A row's peaks (min, max per peak; CPU copy), for the Canvas2D backend. */
  rowData(row: number): Float32Array | null {
    return this.slots[row]?.data ?? null;
  }

  /** Binds the texture to `unit`, uploading new rows. */
  bind(ctx: GlContext, unit: number): void {
    const gl = ctx.gl!;
    gl.activeTexture(gl.TEXTURE0 + unit);
    if (this.generation !== ctx.generation) {
      this.texture = gl.createTexture();
      gl.bindTexture(gl.TEXTURE_2D, this.texture);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RG32F, TILE_PEAKS, TILE_ROWS, 0, gl.RG, gl.FLOAT, null);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
      this.generation = ctx.generation;
      this.slots.forEach((s, i) => s.tile && this.pending.add(i));
    } else {
      gl.bindTexture(gl.TEXTURE_2D, this.texture);
    }
    for (const i of this.pending) uploadRow(gl, i, this.slots[i]!.data);
    this.pending.clear();
  }
}

/** Min/max over the channels per peak; past the tile's peaks: empty (max < min). */
function fillRow(out: Float32Array, t: PeakData): void {
  const n = Math.min(TILE_PEAKS, t.max[0]?.length ?? 0);
  for (let i = 0; i < TILE_PEAKS; i++) {
    let mn = 1e9;
    let mx = -1e9;
    if (i < n) {
      for (let ch = 0; ch < t.max.length; ch++) {
        const a = t.min[ch]![i]!;
        const b = t.max[ch]![i]!;
        if (a < mn) mn = a;
        if (b > mx) mx = b;
      }
    }
    out[i * 2] = mn;
    out[i * 2 + 1] = mx;
  }
}

function uploadRow(gl: GL, row: number, data: Float32Array): void {
  gl.pixelStorei(gl.UNPACK_ALIGNMENT, 4);
  gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, row, TILE_PEAKS, 1, gl.RG, gl.FLOAT, data);
}
