/**
 * The arranger's instanced pipelines and the writers that fill their instance data.
 *
 * - `rect`: SDF rounded rectangles: fills, borders (stroke), CSS-like blurred shadows
 *   (glow), clipped by a rounded clip box. Clip bodies, title bands, selection glow and rim,
 *   edge handles, loop seams, and any overlay a layer wants.
 * - `text`: text sprites and icons from the atlas (`glyphAtlas.ts`), 1:1 at device pixels,
 *   clipped horizontally.
 * - `note`: MIDI note-preview rectangles, placed in the clip body like `drawNotes`.
 * - `wave`: waveform envelope spans sampled from the peak tile texture (`waveTiles.ts`).
 *
 * All positions are in lane space (see shaders.ts): a beat plus px offsets in x, offsets
 * from the row top (`a + b * rowHeight`) in y.
 */

import type { Rgba } from "./palette";
import { NOTE_FS, NOTE_VS, RECT_FS, RECT_VS, TEXT_FS, TEXT_VS, WAVE_FS, WAVE_VS } from "./shaders";
import { Pipeline } from "./glContext";

const v4 = (name: string) => ({ name, size: 4 });

export const rectPipeline = () =>
  new Pipeline("rect", RECT_VS, RECT_FS, [v4("i_x"), v4("i_p"), v4("i_y"), v4("i_fill"), v4("i_stroke"), v4("i_q"), v4("i_cx"), v4("i_cy")]);
export const textPipeline = () => new Pipeline("text", TEXT_VS, TEXT_FS, [v4("i_pos"), v4("i_size"), v4("i_uv"), v4("i_color"), v4("i_cx"), v4("i_c2")]);
export const notePipeline = () => new Pipeline("note", NOTE_VS, NOTE_FS, [v4("i_n"), v4("i_c"), v4("i_color"), v4("i_o")]);
export const wavePipeline = () => new Pipeline("wave", WAVE_VS, WAVE_FS, [v4("i_a"), v4("i_b"), v4("i_c"), v4("i_color")]);

export const RECT_STRIDE = 32;
export const TEXT_STRIDE = 24;
export const NOTE_STRIDE = 16;
export const WAVE_STRIDE = 16;

/** Rect flags (`i_p.w`). */
export const RectFlag = {
  /** The clip box's start / end edge handle (25% wide, clamped to 3–6 px, full height: hitTest.ts `handleWidth`). */
  HandleStart: 1,
  HandleEnd: 2,
  /** Clipped to the lane (row) instead of the clip box: glows. */
  ClipLane: 4,
  /** Not clipped. */
  NoClip: 8,
  /** Edges snapped to device pixels (crisp box edges, like layout boxes). */
  Snap: 16,
} as const;

/** Clip effects (CSS filter + opacity of the whole clip): see `clipFx` in shaders.ts. */
export const Fx = { None: 0, Muted: 1, Frozen: 2 } as const;
export type FxCode = (typeof Fx)[keyof typeof Fx];

/** A growable Float32 array of instances. */
export class InstanceWriter {
  data: Float32Array;
  length = 0;
  constructor(
    readonly stride: number,
    capacity = 64,
  ) {
    this.data = new Float32Array(capacity * stride);
  }
  get count(): number {
    return this.length / this.stride;
  }
  /** Room for one more instance; returns its offset. */
  next(): number {
    if (this.length + this.stride > this.data.length) {
      const grown = new Float32Array(Math.max(this.data.length * 2, this.stride * 64));
      grown.set(this.data.subarray(0, this.length));
      this.data = grown;
    }
    const at = this.length;
    this.length += this.stride;
    return at;
  }
  /** The written instances (a copy sized to fit). */
  take(): Float32Array {
    return this.data.slice(0, this.length);
  }
  clear(): void {
    this.length = 0;
  }
}

/** Horizontal extent of a box: `[beat0 * ppb + px0, max(x0 + minW, beat1 * ppb + px1)]`. */
export interface XSpec {
  b0: number;
  b1: number;
  p0?: number;
  p1?: number;
  minW?: number;
}

/** Vertical extent in a row: `y0 = a0 + b0 * h`, `y1 = a1 + b1 * h` (`h`: main lane height). */
export interface YSpec {
  a0: number;
  f0?: number;
  a1: number;
  f1?: number;
}

export interface RectSpec {
  x: XSpec;
  y: YSpec;
  radius?: number;
  fill?: Rgba;
  stroke?: Rgba;
  strokeWidth?: number;
  /** CSS box-shadow blur (px): draws the rect's shadow in `fill`. */
  blur?: number;
  flags?: number;
  fx?: FxCode;
  /** The clip's opacity (dragged 0.85, ghost 0.45): applied like CSS `opacity` on the clip. */
  alpha?: number;
  /** Clip box (default: unclipped unless `flags` says otherwise). */
  clip?: { x: XSpec; y: YSpec; radius?: number };
}

const TRANSPARENT: Rgba = [0, 0, 0, 0];

export function writeRect(w: InstanceWriter, r: RectSpec): void {
  const o = w.next();
  const dd = w.data;
  const put4 = (i: number, a: number, b: number, c: number, e: number) => {
    dd[o + i] = a;
    dd[o + i + 1] = b;
    dd[o + i + 2] = c;
    dd[o + i + 3] = e;
  };
  const col = (i: number, c: Rgba) => put4(i, c[0], c[1], c[2], c[3]);
  const flags = (r.flags ?? 0) | (r.clip ? 0 : r.flags && r.flags & RectFlag.ClipLane ? 0 : RectFlag.NoClip) | ((r.fx ?? 0) << 5);
  put4(0, r.x.b0, r.x.b1, r.x.p0 ?? 0, r.x.p1 ?? 0);
  put4(4, r.x.minW ?? 0, r.radius ?? 0, r.blur ?? 0, flags);
  put4(8, r.y.a0, r.y.f0 ?? 0, r.y.a1, r.y.f1 ?? 0);
  col(12, r.fill ?? TRANSPARENT);
  col(16, r.stroke ?? TRANSPARENT);
  const c = r.clip;
  put4(20, r.strokeWidth ?? 0, c?.x.minW ?? 0, c?.radius ?? 0, r.alpha ?? 1);
  put4(24, c?.x.b0 ?? 0, c?.x.b1 ?? 0, c?.x.p0 ?? 0, c?.x.p1 ?? 0);
  put4(28, c?.y.a0 ?? 0, c?.y.f0 ?? 0, c?.y.a1 ?? 0, c?.y.f1 ?? 0);
}

export interface GlyphSpec {
  /** Anchor beat and px offset of the quad's left edge; y offset of its top from the row top. */
  beat: number;
  px: number;
  y: number;
  width: number;
  height: number;
  uv: readonly [number, number, number, number];
  /** 0: text sprite, 1: icon (both coverage masks). */
  mode: 0 | 1;
  color: Rgba;
  fx?: FxCode;
  alpha?: number;
  /** Visible horizontal range (a clip box x spec). */
  clip: XSpec;
}

export function writeGlyph(w: InstanceWriter, g: GlyphSpec): void {
  const o = w.next();
  const d = w.data;
  d[o] = g.beat;
  d[o + 1] = g.px;
  d[o + 2] = g.y;
  d[o + 3] = 0;
  d[o + 4] = g.width;
  d[o + 5] = g.height;
  d[o + 6] = g.mode;
  d[o + 7] = g.fx ?? 0;
  d.set(g.uv, o + 8);
  d.set(g.color, o + 12);
  d[o + 16] = g.clip.b0;
  d[o + 17] = g.clip.b1;
  d[o + 18] = g.clip.p0 ?? 0;
  d[o + 19] = g.clip.p1 ?? 0;
  d[o + 20] = g.clip.minW ?? 0;
  d[o + 21] = g.alpha ?? 1;
  d[o + 22] = 0;
  d[o + 23] = 0;
}

/**
 * A note of a clip's preview: `alpha` is the note's own opacity (velocity, muted), `opacity`
 * the clip's (dragged, ghost).
 */
export function writeNote(
  w: InstanceWriter,
  t0: number,
  t1: number,
  pitchPos: number,
  rows: number,
  box: { b0: number; b1: number; minW: number },
  fx: number,
  color: Rgba,
  alpha: number,
  opacity = 1,
): void {
  const o = w.next();
  const d = w.data;
  d[o] = t0;
  d[o + 1] = t1;
  d[o + 2] = pitchPos;
  d[o + 3] = rows;
  d[o + 4] = box.b0;
  d[o + 5] = box.b1;
  d[o + 6] = box.minW;
  d[o + 7] = fx;
  d[o + 8] = color[0];
  d[o + 9] = color[1];
  d[o + 10] = color[2];
  d[o + 11] = color[3] * alpha;
  d[o + 12] = opacity;
  d[o + 13] = 0;
  d[o + 14] = 0;
  d[o + 15] = 0;
}

export function writeWave(
  w: InstanceWriter,
  t0: number,
  t1: number,
  u0: number,
  u1: number,
  row: number,
  gain: number,
  fx: number,
  alpha: number,
  box: { b0: number; b1: number; minW: number },
  color: Rgba,
): void {
  const o = w.next();
  const d = w.data;
  d[o] = t0;
  d[o + 1] = t1;
  d[o + 2] = u0;
  d[o + 3] = u1;
  d[o + 4] = row;
  d[o + 5] = gain;
  d[o + 6] = fx;
  d[o + 7] = alpha;
  d[o + 8] = box.b0;
  d[o + 9] = box.b1;
  d[o + 10] = box.minW;
  d[o + 11] = 0;
  d.set(color, o + 12);
}
