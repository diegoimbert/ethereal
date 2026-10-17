/**
 * The Canvas2D backend of the arranger renderer (see backend.ts): the fallback for browsers
 * without a working WebGL2. It paints the same instance data as the WebGL pipelines, read on
 * the CPU, with what Canvas2D has:
 *
 * - `rect`: rounded rect fills and inner strokes, clipped to their clip box (as per-corner
 *   radii where they can be, else a clip path); the glow (`blur`) is a canvas shadow
 *   (`shadowBlur` is a Gaussian of sigma blur / 2, like CSS `box-shadow`) outside the box.
 * - `text`: sprites of the same atlas (`glyphAtlas.ts`: the browser's own text rasters),
 *   tinted once per ink into small canvases and drawn 1:1 at device pixels.
 * - `note`: plain rects.
 * - `wave`: the envelope of the same peak tiles (`waveTiles.ts` CPU rows) as a path per span,
 *   one point per css px; full-scale marks as rects.
 * - the layers' `fade` (clipFades) and `shape` (automation, comping) pipelines: paths.
 *
 * Simpler than the shaders where it doesn't show: no analytic antialiasing (Canvas2D's own),
 * hatch and dashes drawn as strokes. Pipelines it doesn't know are skipped (and listed once
 * on the console).
 *
 * Without a 2D context (jsdom), it draws nothing but the renderer still runs: hit tests and
 * the test API work.
 */

import { mainHeight, type Row } from "../layout";
import { CLIP_INSET, CLIP_TITLE } from "../laneBodyGeometry";
import type { FrameSetup, RenderBackend } from "./backend";
import type { Batch, Pipeline, RenderContext } from "./glContext";
import type { GlyphAtlas } from "./glyphAtlas";
import type { Rgba } from "./palette";
import { RectFlag } from "./pipelines";
import type { WaveTiles } from "./waveTiles";
import { FadeMode } from "./layers/fadePipeline";

/** Batches and scenes keep their CPU copy only (no GPU objects). */
class Canvas2DContext implements RenderContext {
  readonly gl = null;
  readonly generation = 0;
}

/** The current lane's geometry (css px from the canvas's top-left). */
interface Lane {
  c: CanvasRenderingContext2D;
  f: FrameSetup;
  /** Row top on the canvas, main lane height. */
  top: number;
  h: number;
}

type Painter = (lane: Lane, p: Pipeline, d: Float32Array, first: number, count: number) => void;

type Color = [number, number, number, number];

const BIG = 1e6;
/** Far enough to keep a shadow's caster off the canvas (css px). */
const SHADOW_OFFSET = 50_000;
/** Tinted sprites kept (each a small canvas). */
const SPRITE_CACHE = 4096;

export class Canvas2DBackend implements RenderBackend {
  readonly kind = "canvas2d" as const;
  readonly ctx = new Canvas2DContext();
  private c: CanvasRenderingContext2D | null = null;
  private frame: FrameSetup | null = null;
  private painters: Record<string, Painter>;
  private sprites = new Map<string, CanvasImageSource>();
  private unknown = new Set<string>();
  private restoreListeners = new Set<() => void>();
  private loseListeners = new Set<() => void>();

  constructor(
    private canvas: HTMLCanvasElement,
    private atlas: GlyphAtlas,
    private tiles: WaveTiles,
  ) {
    try {
      this.c = canvas.getContext("2d", { alpha: true });
    } catch {
      this.c = null;
    }
    canvas.addEventListener("contextlost", this.onLost);
    canvas.addEventListener("contextrestored", this.onRestored);
    this.painters = {
      rect: (l, _p, d, a, n) => this.rects(l, d, a, n),
      text: (l, _p, d, a, n) => this.text(l, d, a, n),
      note: (l, _p, d, a, n) => notes(l, d, a, n),
      wave: (l, _p, d, a, n) => this.waves(l, d, a, n),
      fade: (l, p, d, a, n) => fades(l, p.params.radius ?? 6, d, a, n),
      shape: (l, _p, d, a, n) => shapes(l, d, a, n),
    };
  }

  private onLost = (e: Event) => {
    e.preventDefault();
    for (const l of this.loseListeners) l();
  };

  private onRestored = () => {
    this.sprites.clear();
    for (const l of this.restoreListeners) l();
  };

  /** Always: without a 2D context it still hit-tests (and draws nothing). */
  get ok(): boolean {
    return true;
  }

  get ready(): boolean {
    return true;
  }

  begin(f: FrameSetup): void {
    this.frame = f;
    const c = this.c;
    if (!c) return;
    c.setTransform(1, 0, 0, 1, 0, 0);
    c.clearRect(0, 0, f.width, f.height);
    c.setTransform(f.dpr, 0, 0, f.dpr, 0, 0);
    c.imageSmoothingEnabled = false;
    c.globalAlpha = 1;
    c.globalCompositeOperation = "source-over";
  }

  draw(p: Pipeline, batch: Batch, row: Row, first: number, count: number): boolean {
    const f = this.frame;
    if (!f || count <= 0) return false;
    const paint = this.painters[p.name];
    if (!paint) {
      if (!this.unknown.has(p.name)) {
        this.unknown.add(p.name);
        console.warn(`arranger canvas2d: no painter for the "${p.name}" pipeline (skipped)`);
      }
      return false;
    }
    if (!this.c) return true;
    const n = Math.min(count, batch.count - first);
    if (n <= 0) return false;
    const lane: Lane = { c: this.c, f, top: row.y - f.originY, h: mainHeight(row) };
    this.c.save();
    paint(lane, p, batch.data, first, n);
    this.c.restore();
    return true;
  }

  end(): void {
    this.frame = null;
  }

  readPixels(x: number, y: number, w: number, h: number): Uint8Array | null {
    if (!this.c) return null;
    const d = this.c.getImageData(x, y, w, h).data;
    return new Uint8Array(d.buffer, d.byteOffset, d.byteLength);
  }

  frameContext() {
    return { gl: null, ctx2d: this.c };
  }

  onRestore(fn: () => void): () => void {
    this.restoreListeners.add(fn);
    return () => this.restoreListeners.delete(fn);
  }

  onLose(fn: () => void): () => void {
    this.loseListeners.add(fn);
    return () => this.loseListeners.delete(fn);
  }

  simulateLoss(): boolean {
    return false;
  }

  dispose(): void {
    this.canvas.removeEventListener("contextlost", this.onLost);
    this.canvas.removeEventListener("contextrestored", this.onRestored);
    this.sprites.clear();
    this.restoreListeners.clear();
    this.loseListeners.clear();
  }

  // ── rect ──────────────────────────────────────────────────────────────────────────────

  private rects(l: Lane, d: Float32Array, first: number, n: number): void {
    const { c, f } = l;
    const bg = f.palette.bg;
    for (let i = first; i < first + n; i++) {
      const o = i * 32;
      const flags = Math.round(d[o + 7]!);
      const fx = flags >> 5;
      const alpha = d[o + 23]!;
      // The clip box, and the rect.
      const [cx0, cx1] = boxX(f, d[o + 24]!, d[o + 25]!, d[o + 26]!, d[o + 27]!, d[o + 21]!);
      let clip: Box | null = { x0: cx0, y0: rowY(l, d[o + 28]!, d[o + 29]!), x1: cx1, y1: rowY(l, d[o + 30]!, d[o + 31]!) };
      let clipR = d[o + 22]!;
      let [x0, x1] = boxX(f, d[o]!, d[o + 1]!, d[o + 2]!, d[o + 3]!, d[o + 4]!);
      const handle = Math.max(3, Math.min(6, 0.25 * (cx1 - cx0))); // hitTest.ts handleWidth
      if (flags & RectFlag.HandleStart) [x0, x1] = [cx0, cx0 + handle];
      if (flags & RectFlag.HandleEnd) [x0, x1] = [cx1 - handle, cx1];
      let r: Box = { x0, y0: rowY(l, d[o + 8]!, d[o + 9]!), x1, y1: rowY(l, d[o + 10]!, d[o + 11]!) };
      if (flags & RectFlag.Snap) {
        r = snapBox(r, f.dpr);
        clip = snapBox(clip, f.dpr);
      }
      if (flags & RectFlag.ClipLane) {
        clip = { x0: -BIG, y0: rowY(l, 0, 0), x1: BIG, y1: rowY(l, 0, 1) };
        clipR = 0;
      }
      if (flags & RectFlag.NoClip) clip = null;
      const radius = d[o + 5]!;
      const blur = d[o + 6]!;
      const grow = blur > 0 ? blur * 1.5 + 1 : 1;
      // Off the canvas.
      if (r.x1 + grow < 0 || r.x0 - grow > f.cssWidth || r.y1 + grow < 0 || r.y0 - grow > f.cssHeight) continue;
      if (r.x1 <= r.x0 || r.y1 <= r.y0) continue;

      if (blur > 0) {
        // The glow: a shadow outside the box (CSS box-shadow), within the clip box.
        const fill = fxColor(d, o + 12, fx, 1, bg);
        const a = fill[3] * alpha;
        if (a <= 0) continue;
        c.save();
        c.beginPath();
        if (clip) c.rect(clip.x0, clip.y0, clip.x1 - clip.x0, clip.y1 - clip.y0);
        else c.rect(-BIG, -BIG, 2 * BIG, 2 * BIG);
        roundRectPath(c, r, radius);
        c.clip("evenodd");
        c.shadowColor = css(fill, a);
        c.shadowBlur = blur * f.dpr;
        c.shadowOffsetX = SHADOW_OFFSET * f.dpr;
        c.fillStyle = "black"; // eth-allow-hardcoded: the shadow's caster, off the canvas
        c.beginPath();
        roundRectPath(c, { x0: r.x0 - SHADOW_OFFSET, y0: r.y0, x1: r.x1 - SHADOW_OFFSET, y1: r.y1 }, radius);
        c.fill();
        c.restore();
        continue;
      }
      const fill = fxColor(d, o + 12, fx, alpha, bg);
      const sw = d[o + 20]!;
      const stroke = sw > 0 ? fxColor(d, o + 16, fx, alpha, bg) : null;
      if (fill[3] <= 0 && (!stroke || stroke[3] <= 0)) continue;
      const shape = clippedShape(r, radius, clip, clipR);
      if (shape === "empty") continue;
      if (shape === null) {
        // A shape the clip box cuts through a rounded corner: a real clip.
        c.save();
        c.beginPath();
        roundRectPath(c, clip!, clipR);
        c.clip();
        paintRect(c, r, [radius, radius, radius, radius], fill, stroke, sw);
        c.restore();
      } else {
        paintRect(c, shape.box, shape.radii, fill, stroke, sw);
      }
    }
  }

  // ── text ──────────────────────────────────────────────────────────────────────────────

  private text(l: Lane, d: Float32Array, first: number, n: number): void {
    const { c, f } = l;
    const dpr = f.dpr;
    for (let i = first; i < first + n; i++) {
      const o = i * 24;
      const x0 = snap(bx(f, d[o]!) + d[o + 1]!, dpr);
      const y0 = snap(l.top + d[o + 2]!, dpr);
      const w = d[o + 4]!;
      const h = d[o + 5]!;
      const [cx0, cx1] = boxX(f, d[o + 16]!, d[o + 17]!, d[o + 18]!, d[o + 19]!, d[o + 20]!);
      if (!(x0 < cx1 && x0 + w > cx0) || x0 > f.cssWidth || x0 + w < 0 || y0 > f.cssHeight || y0 + h < 0) continue;
      const color = fxColor(d, o + 12, d[o + 7]!, d[o + 21]!, f.palette.bg);
      if (color[3] <= 0) continue;
      const u0 = d[o + 8]!;
      const v0 = d[o + 9]!;
      const sw = Math.round(d[o + 10]! - u0);
      const sh = Math.round(d[o + 11]! - v0);
      if (sw <= 0 || sh <= 0) continue;
      const sprite = this.sprite(u0, v0, sw, sh, color);
      if (!sprite) continue;
      // Clipped horizontally to the clip box, at whole device pixels.
      const s0 = Math.max(0, Math.round((Math.max(x0, cx0) - x0) * dpr));
      const s1 = Math.min(sw, Math.round((Math.min(x0 + w, cx1) - x0) * dpr));
      if (s1 <= s0) continue;
      c.globalAlpha = color[3];
      c.drawImage(sprite, s0, 0, s1 - s0, sh, x0 + s0 / dpr, y0, (s1 - s0) / dpr, sh / dpr);
    }
    c.globalAlpha = 1;
  }

  /** The atlas sprite at (`u`, `v`) tinted in `color`'s rgb (alpha: the coverage). */
  private sprite(u: number, v: number, w: number, h: number, color: Color): CanvasImageSource | null {
    const r = Math.round(color[0] * 255);
    const g = Math.round(color[1] * 255);
    const b = Math.round(color[2] * 255);
    const key = `${u},${v},${w},${h},${this.atlas.stamp(u, v)},${r},${g},${b}`;
    const hit = this.sprites.get(key);
    if (hit) return hit;
    const atlas = this.atlas.pixels;
    if (v + h > atlas.height || u + w > atlas.width) return null;
    const canvas = makeCanvas(w, h);
    const sc = canvas?.getContext("2d") as CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D | null | undefined;
    if (!canvas || !sc) return null;
    const img = sc.createImageData(w, h);
    const px = img.data;
    for (let y = 0; y < h; y++) {
      const row = (v + y) * atlas.width + u;
      for (let x = 0; x < w; x++) {
        const k = (y * w + x) * 4;
        px[k] = r;
        px[k + 1] = g;
        px[k + 2] = b;
        px[k + 3] = atlas.data[row + x]!;
      }
    }
    sc.putImageData(img, 0, 0);
    if (this.sprites.size >= SPRITE_CACHE) this.sprites.clear();
    this.sprites.set(key, canvas);
    return canvas;
  }

  // ── wave ──────────────────────────────────────────────────────────────────────────────

  private waves(l: Lane, d: Float32Array, first: number, n: number): void {
    const { c, f } = l;
    const top = l.top + CLIP_INSET + CLIP_TITLE;
    const bodyH = l.h - 2 * CLIP_INSET - CLIP_TITLE;
    if (bodyH <= 0) return;
    const mid = bodyH * 0.5;
    const mark = Math.min(2, mid);
    const xs: number[] = [];
    const tops: number[] = [];
    const bottoms: number[] = [];
    for (let i = first; i < first + n; i++) {
      const o = i * 16;
      const x0 = bx(f, d[o]!);
      const x1 = bx(f, d[o + 1]!);
      const u0 = d[o + 2]!;
      const u1 = d[o + 3]!;
      const peaks = this.tiles.rowData(Math.round(d[o + 4]!));
      if (!peaks) continue;
      const gain = d[o + 5]!;
      const [cx0, cx1] = boxX(f, d[o + 8]!, d[o + 9]!, 0, 0, d[o + 10]!);
      const lo = Math.max(x0, cx0, 0);
      const hi = Math.min(x1, cx1, f.cssWidth);
      if (hi <= lo) continue;
      const color = fxColor(d, o + 12, d[o + 6]!, d[o + 7]!, f.palette.bg);
      const over = fxColor(f.palette.over, 0, d[o + 6]!, d[o + 7]!, f.palette.bg);
      const du = (u1 - u0) / Math.max(1e-6, x1 - x0);
      const half = 0.5 * Math.abs(du);
      xs.length = tops.length = bottoms.length = 0;
      const marks: Array<[number, number]> = [];
      const flush = () => {
        if (!xs.length) return;
        c.beginPath();
        c.moveTo(xs[0]!, top + tops[0]!);
        for (let k = 0; k < xs.length; k++) c.lineTo(xs[k]! + 1, top + tops[k]!);
        for (let k = xs.length - 1; k >= 0; k--) {
          c.lineTo(xs[k]! + 1, top + bottoms[k]!);
          c.lineTo(xs[k]!, top + bottoms[k]!);
        }
        c.closePath();
        c.fill();
        xs.length = tops.length = bottoms.length = 0;
      };
      c.fillStyle = css(color, color[3]);
      // One column per css px, from the span's first whole px.
      for (let x = Math.floor(lo); x < hi; x++) {
        const xc = x + 0.5;
        const u = u0 + (xc - x0) * du;
        const i0 = clampInt(Math.floor(u - half), 0, 1023);
        const i1 = clampInt(Math.floor(u + half), 0, 1023);
        const stride = Math.max(1, ((i1 - i0 + 1) / 24) | 0);
        let mn = 1e9;
        let mx = -1e9;
        for (let k = i0; k <= i1; k += stride) {
          const a = peaks[k * 2]!;
          const b = peaks[k * 2 + 1]!;
          if (a < mn) mn = a;
          if (b > mx) mx = b;
        }
        if (mx < mn) {
          flush();
          continue;
        }
        const hiV = mx * gain;
        const loV = mn * gain;
        const t = mid - clamp(hiV, -1, 1) * mid;
        let b = Math.max(mid - clamp(loV, -1, 1) * mid, t + 1);
        // The clip box's rounded bottom corners.
        const edge = Math.min(xc - cx0, cx1 - xc);
        if (edge < 6) b = Math.min(b, bodyH - (6 - Math.sqrt(Math.max(0, 36 - (6 - edge) * (6 - edge)))));
        xs.push(x);
        tops.push(t);
        bottoms.push(b);
        if (hiV > 1) marks.push([x, 0]);
        if (loV < -1) marks.push([x, bodyH - mark]);
      }
      flush();
      if (marks.length) {
        c.fillStyle = css(over, over[3]);
        for (const [x, y] of marks) c.fillRect(x, top + y, 1, mark);
      }
    }
  }
}

// ── geometry ──────────────────────────────────────────────────────────────────────────────

interface Box {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

const bx = (f: FrameSetup, beat: number) => (beat - f.scrollBeats) * f.pxPerBeat;
const rowY = (l: Lane, a: number, frac: number) => l.top + a + frac * l.h;
const snap = (v: number, dpr: number) => Math.floor(v * dpr + 0.5) / dpr;
const clamp = (v: number, lo: number, hi: number) => (v < lo ? lo : v > hi ? hi : v);
const clampInt = (v: number, lo: number, hi: number) => (v < lo ? lo : v > hi ? hi : v);

/** A box's x extent: `[x0, max(x0 + minW, x1)]` (see `boxX` in shaders.ts). */
function boxX(f: FrameSetup, b0: number, b1: number, p0: number, p1: number, minW: number): [number, number] {
  const x0 = bx(f, b0) + p0;
  return [x0, Math.max(x0 + minW, bx(f, b1) + p1)];
}

function snapBox(b: Box, dpr: number): Box {
  return { x0: snap(b.x0, dpr), y0: snap(b.y0, dpr), x1: snap(b.x1, dpr), y1: snap(b.y1, dpr) };
}

/** Corner radii (top-left, top-right, bottom-right, bottom-left) clamped to the box. */
function fitRadii(b: Box, radii: readonly number[]): number[] {
  const m = Math.max(0, Math.min((b.x1 - b.x0) / 2, (b.y1 - b.y0) / 2));
  return radii.map((r) => Math.max(0, Math.min(r, m)));
}

function roundRectPath(c: CanvasRenderingContext2D, b: Box, radius: number | readonly number[]): void {
  const w = b.x1 - b.x0;
  const h = b.y1 - b.y0;
  if (w <= 0 || h <= 0) return;
  const radii = fitRadii(b, typeof radius === "number" ? [radius, radius, radius, radius] : radius);
  if (radii.every((r) => r <= 0.01)) c.rect(b.x0, b.y0, w, h);
  else c.roundRect(b.x0, b.y0, w, h, radii);
}

/**
 * `r` (corner radius `radius`) clipped to `clip` (corner radius `clipR`) as one rounded
 * rect when it is one: the intersection, with the clip's radius at the corners they share.
 * Null when the clip cuts through one of its rounded corners elsewhere (needs a clip path).
 */
export function clippedShape(r: Box, radius: number, clip: Box | null, clipR: number): { box: Box; radii: number[] } | "empty" | null {
  if (!clip) return { box: r, radii: [radius, radius, radius, radius] };
  const box = { x0: Math.max(r.x0, clip.x0), y0: Math.max(r.y0, clip.y0), x1: Math.min(r.x1, clip.x1), y1: Math.min(r.y1, clip.y1) };
  if (box.x1 <= box.x0 || box.y1 <= box.y0) return "empty";
  const eps = 0.01;
  const inside = box.x0 - r.x0 < eps && r.x1 - box.x1 < eps && box.y0 - r.y0 < eps && r.y1 - box.y1 < eps;
  if (!inside && radius > eps) return null;
  const cr = fitRadii(clip, [clipR])[0]!;
  const own = inside ? radius : 0;
  // Each corner: [x, y, the clip corner's x, y].
  const corners: Array<[number, number, number, number]> = [
    [box.x0, box.y0, clip.x0, clip.y0],
    [box.x1, box.y0, clip.x1, clip.y0],
    [box.x1, box.y1, clip.x1, clip.y1],
    [box.x0, box.y1, clip.x0, clip.y1],
  ];
  const radii: number[] = [];
  for (const [x, y, kx, ky] of corners) {
    const dx = cr - Math.abs(x - kx);
    const dy = cr - Math.abs(y - ky);
    if (cr <= eps || dx <= 0 || dy <= 0) radii.push(own);
    else if (Math.abs(x - kx) < eps && Math.abs(y - ky) < eps) radii.push(Math.max(own, cr));
    else if (dx * dx + dy * dy <= cr * cr) radii.push(own);
    else return null;
  }
  return { box, radii };
}

function paintRect(c: CanvasRenderingContext2D, b: Box, radii: number[], fill: Color, stroke: Color | null, sw: number): void {
  if (fill[3] > 0) {
    c.fillStyle = css(fill, fill[3]);
    c.beginPath();
    roundRectPath(c, b, radii);
    c.fill();
  }
  if (stroke && stroke[3] > 0 && sw > 0) {
    // An inner stroke: the path inset by half its width.
    const h = sw / 2;
    c.strokeStyle = css(stroke, stroke[3]);
    c.lineWidth = sw;
    c.beginPath();
    roundRectPath(
      c,
      { x0: b.x0 + h, y0: b.y0 + h, x1: b.x1 - h, y1: b.y1 - h },
      radii.map((r) => Math.max(0, r - h)),
    );
    c.stroke();
  }
}

// ── colors ────────────────────────────────────────────────────────────────────────────────

/**
 * The color at `d[o..o+4]` with the clip effects of `clipFx` (shaders.ts): muted / frozen
 * desaturate and fade, the clip's opacity mixes into the background; alpha unchanged.
 */
function fxColor(d: ArrayLike<number>, o: number, fx: number, opacity: number, bg: Rgba): Color {
  let r = d[o]!;
  let g = d[o + 1]!;
  let b = d[o + 2]!;
  let op = opacity;
  if (fx > 0.5) {
    const s = fx < 1.5 ? 0.15 : 0.35;
    const r2 = r * (0.213 + 0.787 * s) + g * (0.715 - 0.715 * s) + b * (0.072 - 0.072 * s);
    const g2 = r * (0.213 - 0.213 * s) + g * (0.715 + 0.285 * s) + b * (0.072 - 0.072 * s);
    const b2 = r * (0.213 - 0.213 * s) + g * (0.715 - 0.715 * s) + b * (0.072 + 0.928 * s);
    r = r2;
    g = g2;
    b = b2;
    op *= fx < 1.5 ? 0.55 : 0.75;
  }
  if (op < 1) {
    r = bg[0] + (r - bg[0]) * op;
    g = bg[1] + (g - bg[1]) * op;
    b = bg[2] + (b - bg[2]) * op;
  }
  return [r, g, b, d[o + 3]!];
}

const byte = (v: number) => Math.round(clamp(v, 0, 1) * 255);

function css(c: Color, a: number): string {
  // eth-allow-hardcoded: a computed color (tokens already resolved into the instances)
  return `rgba(${byte(c[0])},${byte(c[1])},${byte(c[2])},${clamp(a, 0, 1).toFixed(4)})`;
}

function makeCanvas(w: number, h: number): HTMLCanvasElement | OffscreenCanvas | null {
  if (typeof OffscreenCanvas !== "undefined") return new OffscreenCanvas(w, h);
  if (typeof document === "undefined") return null;
  const el = document.createElement("canvas");
  el.width = w;
  el.height = h;
  return el;
}

// ── note ──────────────────────────────────────────────────────────────────────────────────

function notes(l: Lane, d: Float32Array, first: number, n: number): void {
  const { c, f } = l;
  const top = l.top + CLIP_INSET + CLIP_TITLE;
  const bodyH = l.h - 2 * CLIP_INSET - CLIP_TITLE;
  if (bodyH <= 0) return;
  let style = "";
  for (let i = first; i < first + n; i++) {
    const o = i * 16;
    // Drum rows (negative count): as the note shader (taller hits, a gap between rows).
    const drums = d[o + 3]! < 0;
    const rows = Math.abs(d[o + 3]!);
    const pad = drums ? Math.min(4, bodyH * 0.1) : 0;
    const h = drums ? Math.max(1, Math.min(14, ((bodyH - 2 * pad) / rows) * 0.6)) : Math.max(1, Math.min(6, bodyH / rows));
    const usable = bodyH - 2 * pad - h;
    const y = top + pad + usable - d[o + 2]! * usable;
    const x0 = bx(f, d[o]!);
    const w = Math.max(1, (d[o + 1]! - d[o]!) * f.pxPerBeat - 1);
    const [cx0, cx1] = boxX(f, d[o + 4]!, d[o + 5]!, 0, 0, d[o + 6]!);
    const xa = Math.max(x0, cx0, 0);
    const xb = Math.min(x0 + w, cx1, f.cssWidth);
    const ya = Math.max(y, top);
    const yb = Math.min(y + h, top + bodyH);
    if (xb <= xa || yb <= ya) continue;
    const color = fxColor(d, o + 8, d[o + 7]!, d[o + 12]!, f.palette.bg);
    const s = css(color, color[3]);
    if (s !== style) c.fillStyle = style = s;
    c.fillRect(xa, ya, xb - xa, yb - ya);
  }
}

// ── fade (layers/fadePipeline.ts) ─────────────────────────────────────────────────────────

const HANDLE = 4;

function fadeGain(code: number, tension: number, x: number): number {
  const t = clamp(x, 0, 1);
  if (code < 0.5) return t;
  if (code < 1.5) return Math.sin(t * Math.PI * 0.5);
  return Math.pow(t, Math.pow(4, clamp(tension, -1, 1)));
}

function fades(l: Lane, radius: number, d: Float32Array, first: number, n: number): void {
  const { c, f } = l;
  const dpr = f.dpr;
  const top = l.top + CLIP_INSET + CLIP_TITLE;
  const bodyH = Math.max(0, l.h - 2 * CLIP_INSET - CLIP_TITLE);
  for (let i = first; i < first + n; i++) {
    const o = i * 24;
    const mode = Math.round(d[o + 3]!);
    const [b0, b1] = boxX(f, d[o]!, d[o + 1]!, 0, 0, d[o + 2]!);
    const clip = snapBox({ x0: b0, y0: rowY(l, CLIP_INSET, 0), x1: b1, y1: rowY(l, -CLIP_INSET, 1) }, dpr);
    const fa = bx(f, d[o + 4]!);
    const fb = bx(f, d[o + 5]!);
    const code = d[o + 6]!;
    const tension = d[o + 7]!;
    const va = Math.max(clip.x0, bx(f, d[o + 8]!));
    const vb = Math.min(clip.x1, bx(f, d[o + 9]!));
    const dir = d[o + 10]!;
    const fx = d[o + 20]!;
    const alpha = d[o + 21]!;
    if (vb <= va || clip.x1 < 0 || clip.x0 > f.cssWidth || bodyH <= 0) continue;
    const fill = fxColor(d, o + 12, fx, 1, f.palette.bg);
    const curveY = (x: number) => {
      const u = (x - fa) / Math.max(1e-6, fb - fa);
      const g = dir > 0 ? fadeGain(code, tension, u) : fadeGain(code, tension, 1 - u);
      return top + (1 - clamp(g, 0, 1)) * bodyH;
    };
    /** The curve's points, one per css px (within the canvas). */
    const curve = (): Array<[number, number]> => {
      const pts: Array<[number, number]> = [];
      const x0 = Math.max(fa, -2);
      const x1 = Math.min(fb, f.cssWidth + 2);
      for (let x = x0; ; x = Math.min(x1, x + 1)) {
        pts.push([x, curveY(x)]);
        if (x >= x1) break;
      }
      return pts;
    };
    c.save();
    c.globalAlpha = alpha;
    c.beginPath();
    roundRectPath(c, clip, radius);
    c.clip();
    c.beginPath();
    c.rect(va, clip.y0 - 1, vb - va, clip.y1 - clip.y0 + 2);
    c.clip();
    c.fillStyle = css(fill, fill[3]);
    c.strokeStyle = css(fill, fill[3]);
    if (mode === FadeMode.Tint) {
      if (fb - fa > 0.5) c.fillRect(fa, top, fb - fa, bodyH);
    } else if (mode === FadeMode.Shade) {
      if (fb - fa > 0.5) {
        c.beginPath();
        c.moveTo(fa, top);
        for (const [x, y] of curve()) c.lineTo(x, y);
        c.lineTo(fb, top);
        c.closePath();
        c.fill();
      }
    } else if (mode === FadeMode.Line || mode === FadeMode.Dashed) {
      if (fb - fa > 0.5) {
        c.lineWidth = 1;
        if (mode === FadeMode.Dashed) c.setLineDash([2, 2]);
        c.beginPath();
        curve().forEach(([x, y], k) => (k ? c.lineTo(x, y) : c.moveTo(x, y)));
        c.stroke();
      }
    } else if (mode === FadeMode.Handle || mode === FadeMode.CurveHandle) {
      let r: Box;
      if (mode === FadeMode.Handle) {
        const cx = Math.min(Math.max(fa, clip.x0 + d[o + 22]!), clip.x1 - d[o + 23]!);
        r = { x0: cx - HANDLE / 2, y0: top, x1: cx + HANDLE / 2, y1: top + HANDLE };
      } else {
        if (fb - fa < 8) {
          c.restore();
          continue;
        }
        const cx = 0.5 * (fa + fb);
        const cy = top + (1 - fadeGain(code, tension, 0.5)) * bodyH;
        r = { x0: cx - HANDLE / 2, y0: cy - HANDLE / 2, x1: cx + HANDLE / 2, y1: cy + HANDLE / 2 };
      }
      const rr = d[o + 11]!;
      paintRect(c, r, [rr, rr, rr, rr], fill, fxColor(d, o + 16, fx, 1, f.palette.bg), 1);
    } else if (mode === FadeMode.Hatch) {
      // repeating-linear-gradient(135deg, c 0 2px, transparent 2px 8px): stripes along x + y.
      const step = 8 * Math.SQRT2;
      const s0 = clip.x0 + clip.y0;
      const s1 = clip.x1 + clip.y1;
      c.lineWidth = 2;
      c.beginPath();
      for (let s = s0 + Math.SQRT2; s < s1 + step; s += step) {
        // The line x + y = s across the clip box.
        c.moveTo(s - clip.y0, clip.y0);
        c.lineTo(s - clip.y1, clip.y1);
      }
      c.stroke();
    } else if (mode === FadeMode.Outline) {
      c.lineWidth = 1;
      c.setLineDash([3, 3]);
      c.beginPath();
      roundRectPath(c, { x0: clip.x0 + 0.5, y0: clip.y0 + 0.5, x1: clip.x1 - 0.5, y1: clip.y1 - 0.5 }, Math.max(0, radius - 0.5));
      c.stroke();
    }
    c.restore();
  }
}

// ── shape (layers/shapes.ts) ──────────────────────────────────────────────────────────────

function shapes(l: Lane, d: Float32Array, first: number, n: number): void {
  const { c, f } = l;
  /** The open polyline (same style, consecutive segments joined). */
  let run: { key: string; x: number; y: number } | null = null;
  const endRun = () => {
    if (run) c.stroke();
    run = null;
  };
  let clipKey = "";
  let clipped = false;
  for (let i = first; i < first + n; i++) {
    const o = i * 24;
    let ax = bx(f, d[o]!) + d[o + 1]!;
    let ay = l.top + d[o + 4]!;
    let bxp = bx(f, d[o + 2]!) + d[o + 3]!;
    let by = l.top + d[o + 5]!;
    const r = d[o + 6]!;
    const sw = d[o + 7]!;
    const outline = d[o + 16]! > 0.5;
    const alpha = d[o + 19]!;
    const c0 = l.top + d[o + 20]!;
    const c1 = l.top + d[o + 21]!;
    // The vertical clip range (an automation lane), set when it changes.
    const ck = `${c0},${c1}`;
    if (ck !== clipKey) {
      endRun();
      if (clipped) c.restore();
      c.save();
      clipped = true;
      c.beginPath();
      c.rect(-BIG, c0, 2 * BIG, c1 - c0);
      c.clip();
      clipKey = ck;
    }
    if (outline) {
      endRun();
      bxp = Math.max(bxp, ax + d[o + 18]!);
      const box = { x0: Math.min(ax, bxp), y0: Math.min(ay, by), x1: Math.max(ax, bxp), y1: Math.max(ay, by) };
      if (box.x1 < 0 || box.x0 > f.cssWidth || sw <= 0) continue;
      const h = sw / 2;
      c.globalAlpha = alpha;
      c.strokeStyle = css(rgbaAt(d, o + 12), d[o + 15]!);
      c.lineWidth = sw;
      c.setLineDash(d[o + 17]! > 0 ? [d[o + 17]!, d[o + 17]!] : []);
      c.beginPath();
      roundRectPath(c, { x0: box.x0 + h, y0: box.y0 + h, x1: box.x1 - h, y1: box.y1 - h }, Math.max(0, r - h));
      c.stroke();
      c.setLineDash([]);
      continue;
    }
    const point = Math.abs(bxp - ax) < 1e-3 && Math.abs(by - ay) < 1e-3;
    if (point) {
      endRun();
      if (ax + r < 0 || ax - r > f.cssWidth) continue;
      c.globalAlpha = alpha;
      if (d[o + 11]! > 0) {
        c.fillStyle = css(rgbaAt(d, o + 8), d[o + 11]!);
        c.beginPath();
        c.arc(ax, ay, r, 0, 2 * Math.PI);
        c.fill();
      }
      if (sw > 0 && d[o + 15]! > 0) {
        c.strokeStyle = css(rgbaAt(d, o + 12), d[o + 15]!);
        c.lineWidth = sw;
        c.beginPath();
        c.arc(ax, ay, Math.max(0, r - sw / 2), 0, 2 * Math.PI);
        c.stroke();
      }
      continue;
    }
    // A segment: clipped to the canvas (with room for the caps), as the shader does.
    const pad = r + 2;
    const dx = bxp - ax;
    if (Math.abs(dx) > 1e-3) {
      const t0 = (-pad - ax) / dx;
      const t1 = (f.cssWidth + pad - ax) / dx;
      const ta = clamp(Math.min(t0, t1), 0, 1);
      const tb = clamp(Math.max(t0, t1), 0, 1);
      if (tb <= ta) {
        endRun();
        continue;
      }
      const dy = by - ay;
      [ax, ay, bxp, by] = [ax + dx * ta, ay + dy * ta, ax + dx * tb, ay + dy * tb];
    } else if (ax + pad < 0 || ax - pad > f.cssWidth) {
      endRun();
      continue;
    }
    const key = `${d[o + 8]},${d[o + 9]},${d[o + 10]},${d[o + 11]},${r},${alpha}`;
    const cur = run as { key: string; x: number; y: number } | null;
    if (!cur || cur.key !== key) {
      endRun();
      c.globalAlpha = alpha;
      c.strokeStyle = css(rgbaAt(d, o + 8), d[o + 11]!);
      c.lineWidth = 2 * r;
      c.lineCap = "round";
      c.lineJoin = "round";
      c.beginPath();
      c.moveTo(ax, ay);
    } else if (Math.abs(cur.x - ax) > 0.01 || Math.abs(cur.y - ay) > 0.01) {
      c.moveTo(ax, ay);
    }
    c.lineTo(bxp, by);
    run = { key, x: bxp, y: by };
  }
  endRun();
  if (clipped) c.restore();
}

function rgbaAt(d: Float32Array, o: number): Color {
  return [d[o]!, d[o + 1]!, d[o + 2]!, d[o + 3]!];
}
