/**
 * Runtime text atlas of the arranger: each distinct text run (string, font, ink) rasterized
 * whole by the browser (`textRaster.ts`: Canvas2D `fillText` in the DOM's font, features
 * and gamma) as an alpha coverage sprite, plus small icons (the clip loop icon). Arranger
 * text never scales with the zoom, so sprites are drawn 1:1: a quad of the sprite's device
 * pixel size at a device pixel, each fragment fetching its texel (`texelFetch`), and the
 * coverage blended in sRGB like the browser composites text. Names come out as the DOM
 * (DOM) renderer draws them.
 *
 * - **Layout** (`layout`): the whole string if it fits, else cut at a grapheme cluster with
 *   an ellipsis, like `text-overflow: ellipsis` (widths from `measureText`).
 * - **Packing**: shelves (`ShelfPacker`) in one `R8` texture, uploaded incrementally (the
 *   rows changed since the last bind). When it is full, shelves not used since the last
 *   rebuild of the text are evicted (least recently used first); when every shelf is in
 *   use, `version` is bumped so the text is rebuilt and the shelves still needed marked.
 * - `reset()` (fonts loaded, device pixel ratio changed, context restored) drops every
 *   sprite and bumps `version`; the scene and the layers rebuild their text then.
 */

import type { GL, GlContext } from "./glContext";
import type { Rgba } from "./palette";
import { CanvasTextRasterizer, canonicalInk, type FontSpec, type TextRasterizer } from "./textRaster";
import { renderDpr } from "@/timeline/renderDpr";

export type { FontSpec } from "./textRaster";

/** The atlas texture (`R8`, 4 MB): about 1500 names at 2x, 6000 at 1x. */
const ATLAS_W = 2048;
const ATLAS_H = 2048;
export const ELLIPSIS = "…";
const WHITE: Rgba = [1, 1, 1, 1];

export interface Glyph {
  /** Atlas rect (texels: x0, y0, x1, y1). */
  uv: [number, number, number, number];
  /** Quad size and offset from the pen position at the baseline (css px). */
  width: number;
  height: number;
  left: number;
  top: number;
  advance: number;
  /** 0: text sprite, 1: icon (both alpha coverage). */
  mode: 0 | 1;
}

/** A text run laid out for drawing: sprites at pen x offsets (css px), and its width. */
export interface TextRun {
  glyphs: Array<{ glyph: Glyph; x: number }>;
  width: number;
  /** The string drawn (ellipsized when cut). */
  text: string;
}

/** Lucide `Repeat` (24×24 viewBox), the clip loop icon. */
const REPEAT_PATHS = ["m17 2 4 4-4 4", "M3 11v-1a4 4 0 0 1 4-4h14", "m7 22-4-4 4-4", "M21 13v1a4 4 0 0 1-4 4H3"];

/** A shelf of the atlas: a row of sprites of one height class. */
export interface Shelf {
  y: number;
  h: number;
  /** Next free x. */
  x: number;
  /** `epoch` of the last use of a sprite on it. */
  lastUse: number;
  /** Keys of the sprites on it. */
  keys: string[];
}

/** Height classes of shelves: sprites of close heights share shelves. */
export function shelfClass(h: number): number {
  return h <= 8 ? 8 : Math.ceil(h / 4) * 4;
}

/**
 * Shelf packing in a `width` × `maxHeight` area: a sprite goes on the first shelf of its
 * height class with room, else on a new shelf, else on an emptied (evicted) shelf tall
 * enough. Eviction empties whole shelves.
 */
export class ShelfPacker {
  shelves: Shelf[] = [];
  /** Height used by shelves. */
  top = 0;
  constructor(
    readonly width: number,
    readonly maxHeight: number,
  ) {}

  alloc(w: number, h: number, key: string, epoch: number): { x: number; y: number; shelf: Shelf } | null {
    if (w <= 0 || h <= 0 || w > this.width || h > this.maxHeight) return null;
    const cls = shelfClass(h);
    let empty: Shelf | null = null;
    for (const s of this.shelves) {
      if (s.h === cls && s.x + w <= this.width) return this.place(s, w, key, epoch);
      if (!empty && s.keys.length === 0 && s.h >= cls && s.h <= cls * 2) empty = s;
    }
    if (this.top + cls <= this.maxHeight) {
      const s: Shelf = { y: this.top, h: cls, x: 0, lastUse: epoch, keys: [] };
      this.top += cls;
      this.shelves.push(s);
      return this.place(s, w, key, epoch);
    }
    if (empty) return this.place(empty, w, key, epoch);
    return null;
  }

  private place(s: Shelf, w: number, key: string, epoch: number): { x: number; y: number; shelf: Shelf } {
    const at = { x: s.x, y: s.y, shelf: s };
    s.x += w;
    s.lastUse = epoch;
    s.keys.push(key);
    return at;
  }

  /**
   * Empties the least recently used shelves last used before `epoch` (up to `max`, oldest
   * first): returns the keys of their sprites.
   */
  evict(epoch: number, max = Infinity): string[] {
    const old = this.shelves.filter((s) => s.lastUse < epoch && s.keys.length > 0).sort((a, b) => a.lastUse - b.lastUse);
    const out: string[] = [];
    for (const s of old.slice(0, max)) {
      out.push(...s.keys);
      s.keys = [];
      s.x = 0;
    }
    return out;
  }

  clear(): void {
    this.shelves = [];
    this.top = 0;
  }
}

interface Sprite {
  glyph: Glyph;
  shelf: Shelf;
}

export class GlyphAtlas {
  /** Bumped when sprites placed before may be gone: text instances must be rebuilt. */
  version = 0;
  private readonly w: number;
  private readonly h: number;
  private packer: ShelfPacker;
  private data: Uint8Array;
  private height: number;
  /** Rows changed since the last upload ([dirtyY0, dirtyY1)). */
  private dirtyY0 = Infinity;
  private dirtyY1 = 0;
  /** Each sprite's blit, by its atlas origin (`x,y`): tells a reused spot from its old sprite. */
  private stamps = new Map<string, number>();
  private blits = 0;
  private sprites = new Map<string, Sprite>();
  /** A sprite's use marks its shelf with the current epoch (bumped with `version`). */
  private epoch = 0;
  /** The epoch a full atlas last bumped `version` in to collect its shelves in use. */
  private collectEpoch = -1;
  private texture: WebGLTexture | null = null;
  private textureGeneration = -1;
  /** Rows of the texture allocated (grows with the packed rows; uvs are texels). */
  private textureRows = 0;
  private dpr = 1;
  private widths = new Map<string, number>();
  private fits = new Map<string, string>();
  /** `canonicalInk` of the palette's colors (arrays reused from build to build). */
  private inks = new WeakMap<Rgba, string>();
  readonly raster: TextRasterizer;

  /** `size`: the texture's (texels; tests use a small one). */
  constructor(raster: TextRasterizer = new CanvasTextRasterizer(), size: { width: number; height: number } = { width: ATLAS_W, height: ATLAS_H }) {
    this.raster = raster;
    this.w = size.width;
    this.h = size.height;
    this.packer = new ShelfPacker(this.w, this.h);
    this.height = Math.min(256, this.h);
    this.data = new Uint8Array(this.w * this.height);
    this.reset();
  }

  /** Drops every sprite (font loaded, device pixel ratio changed). */
  reset(dpr = renderDpr()): void {
    this.dpr = dpr;
    this.raster.reset(dpr);
    this.packer.clear();
    this.sprites.clear();
    this.stamps.clear();
    this.widths.clear();
    this.fits.clear();
    this.bump();
  }

  private bump(): void {
    this.version++;
    this.epoch++;
  }

  get devicePixelRatio(): number {
    return this.dpr;
  }

  /** Bytes of the atlas texture. */
  get bytes(): number {
    return this.w * this.textureRows;
  }

  /** Sprites in the atlas. */
  get size(): number {
    return this.sprites.size;
  }

  /** The font's ascent and descent (css px; the browser's, rounded to whole px). */
  metrics(font: FontSpec): { ascent: number; descent: number } {
    return this.raster.metrics(font);
  }

  /**
   * The baseline of a line box `height` px tall at `top` (css px), where the browser paints
   * it: centered by its font's ascent and descent (half-leading), rounded to a whole css px
   * (Chrome paints text at whole css px at any device pixel ratio: measured at 1x and 2x).
   */
  baseline(top: number, height: number, font: FontSpec): number {
    const m = this.raster.metrics(font);
    return Math.round(top + (height - (m.ascent + m.descent)) / 2 + m.ascent);
  }

  /** Width of `text` (css px). */
  measure(text: string, font: FontSpec): number {
    const key = `${font.weight}|${font.size}|${text}`;
    let w = this.widths.get(key);
    if (w === undefined) {
      w = text ? this.raster.measure(text, font) : 0;
      if (this.widths.size > 50_000) this.widths.clear();
      this.widths.set(key, w);
    }
    return w;
  }

  /**
   * `text` as drawn within `maxWidth` css px: whole if it fits, else cut with an ellipsis
   * like `text-overflow: ellipsis` (as many grapheme clusters as fit before the "…"; a
   * space before it stays, as in Chrome).
   */
  fit(text: string, font: FontSpec, maxWidth: number): string {
    if (!text || this.measure(text, font) <= maxWidth + 0.01) return text;
    const key = `${font.weight}|${font.size}|${Math.floor(maxWidth * 4)}|${text}`;
    const hit = this.fits.get(key);
    if (hit !== undefined) return hit;
    const room = maxWidth - this.measure(ELLIPSIS, font);
    const ends = clusterEnds(text);
    // The longest prefix that fits (prefix widths grow with their length).
    let lo = 0;
    let hi = ends.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (this.measure(text.slice(0, ends[mid]!), font) <= room + 0.01) lo = mid;
      else hi = mid - 1;
    }
    const cut = lo > 0 || this.measure(text.slice(0, ends[0]!), font) <= room + 0.01 ? ends[lo]! : 0;
    const out = text.slice(0, cut) + ELLIPSIS;
    if (this.fits.size > 20_000) this.fits.clear();
    this.fits.set(key, out);
    return out;
  }

  /**
   * `text` laid out within `maxWidth` css px (`fit`), drawn in `ink` (the color the text is
   * drawn in: the browser's coverage depends on it; the shader tints the sprite). Spaces
   * collapse as in `white-space: nowrap`. One sprite at x 0; empty when there is no text
   * or the atlas is full.
   */
  layout(text: string, font: FontSpec, maxWidth: number, ink: Rgba = WHITE): TextRun {
    const t = collapseSpaces(text);
    const s = maxWidth === Infinity ? t : this.fit(t, font, maxWidth);
    if (s === t) {
      const glyph = s ? this.sprite(s, font, ink) : null;
      return { glyphs: glyph ? [{ glyph, x: 0 }] : [], width: this.measure(s, font), text: s };
    }
    // Cut: the text, then the ellipsis shaped on its own after it (as Chrome paints it: a
    // run of its own, so contextual forms of the text don't reach it).
    const kept = s.slice(0, -ELLIPSIS.length);
    const x = this.measure(kept, font);
    const glyphs: TextRun["glyphs"] = [];
    const head = kept ? this.sprite(kept, font, ink) : null;
    if (head) glyphs.push({ glyph: head, x: 0 });
    const ell = this.sprite(ELLIPSIS, font, ink);
    if (ell) glyphs.push({ glyph: ell, x });
    return { glyphs, width: x + this.measure(ELLIPSIS, font), text: s };
  }

  /** The sprite of `text` in `ink` (rasterized on first use), or null. */
  sprite(text: string, font: FontSpec, ink: Rgba = WHITE): Glyph | null {
    let color = this.inks.get(ink);
    if (color === undefined) this.inks.set(ink, (color = canonicalInk(ink)));
    const key = `${color}|${font.weight}|${font.size}|${text}`;
    const hit = this.sprites.get(key);
    if (hit) {
      hit.shelf.lastUse = this.epoch;
      return hit.glyph;
    }
    const r = this.raster.draw(text, font, color);
    if (!r) return null;
    const at = this.alloc(r.width, r.height, key);
    if (!at) return null;
    this.blit(r.data, r.width, r.height, at.x, at.y);
    const d = this.dpr;
    const glyph: Glyph = {
      uv: [at.x, at.y, at.x + r.width, at.y + r.height],
      width: r.width / d,
      height: r.height / d,
      left: -r.originX / d,
      top: -r.baseline / d,
      advance: r.advance,
      mode: 0,
    };
    this.sprites.set(key, { glyph, shelf: at.shelf });
    return glyph;
  }

  /** The loop icon as a coverage mask, `size` css px square. */
  repeatIcon(size: number, strokeWidth: number): Glyph {
    return this.icon("repeat", REPEAT_PATHS, size, strokeWidth);
  }

  /**
   * A stroked icon (SVG path data in a 24×24 viewBox, like lucide's) as a coverage mask,
   * `size` css px square; `name` keys it.
   */
  icon(name: string, paths: ReadonlyArray<string>, size: number, strokeWidth: number): Glyph {
    const key = `icon|${name}|${size}|${strokeWidth}`;
    const hit = this.sprites.get(key);
    if (hit) {
      hit.shelf.lastUse = this.epoch;
      return hit.glyph;
    }
    const px = Math.max(1, Math.round(size * this.dpr));
    const at = this.alloc(px, px, key);
    if (!at) return { uv: [0, 0, 0, 0], width: 0, height: 0, left: 0, top: 0, advance: size, mode: 1 };
    this.blit(this.raster.icon(paths, px, strokeWidth), px, px, at.x, at.y);
    const glyph: Glyph = { uv: [at.x, at.y, at.x + px, at.y + px], width: px / this.dpr, height: px / this.dpr, left: 0, top: 0, advance: size, mode: 1 };
    this.sprites.set(key, { glyph, shelf: at.shelf });
    return glyph;
  }

  /** Room for a `w` × `h` sprite: evicts shelves not used since the last rebuild if full. */
  private alloc(w: number, h: number, key: string): { x: number; y: number; shelf: Shelf } | null {
    let at = this.packer.alloc(w, h, key, this.epoch);
    if (at) return at;
    // Shelves not used since the text was last rebuilt: no instance holds their sprites.
    const evicted = this.packer.evict(this.epoch, Math.max(1, this.packer.shelves.length >> 3));
    for (const k of evicted) this.sprites.delete(k);
    at = evicted.length ? this.packer.alloc(w, h, key, this.epoch) : null;
    if (at) return at;
    // Every shelf may be in use: rebuild the text once (it marks the shelves it uses),
    // then evict the others. Not twice in a row (the text in view doesn't fit).
    if (this.collectEpoch !== this.epoch) {
      this.bump();
      this.collectEpoch = this.epoch;
    }
    return null;
  }

  private blit(src: ArrayLike<number>, w: number, h: number, x: number, y: number): void {
    if (y + h > this.height) {
      let rows = this.height;
      while (y + h > rows) rows = Math.min(rows * 2, this.h);
      const grown = new Uint8Array(this.w * rows);
      grown.set(this.data);
      this.data = grown;
      this.height = rows;
    }
    for (let r = 0; r < h; r++) this.data.set((src as Uint8Array).subarray(r * w, r * w + w), (y + r) * this.w + x);
    this.stamps.set(`${x},${y}`, ++this.blits);
    this.dirtyY0 = Math.min(this.dirtyY0, y);
    this.dirtyY1 = Math.max(this.dirtyY1, y + h);
  }

  /**
   * The atlas's coverage (CPU copy, `R8` rows of `width` texels) for the Canvas2D backend,
   * which draws sprites from it without a texture.
   */
  get pixels(): { data: Uint8Array; width: number; height: number } {
    return { data: this.data, width: this.w, height: this.height };
  }

  /**
   * Which blit put the sprite at atlas texel (`x`, `y`) (its uv origin) there: changes when
   * an evicted spot gets another sprite, so caches keyed by uv can tell (0: none).
   */
  stamp(x: number, y: number): number {
    return this.stamps.get(`${x},${y}`) ?? 0;
  }

  /** Binds the atlas texture (uploading the rows changed) to texture unit `unit`. */
  bind(ctx: GlContext, unit: number): void {
    const gl = ctx.gl!;
    if (this.textureGeneration !== ctx.generation) {
      this.texture = gl.createTexture();
      this.textureGeneration = ctx.generation;
      this.textureRows = 0;
    }
    gl.activeTexture(gl.TEXTURE0 + unit);
    gl.bindTexture(gl.TEXTURE_2D, this.texture);
    if (this.textureRows < this.height) {
      // New texture, or the packed rows outgrew it: (re)allocate and upload every row.
      this.textureRows = this.height;
      allocate(gl, this.w, this.height);
      upload(gl, this.data, this.w, 0, this.height);
    } else if (this.dirtyY1 > this.dirtyY0) {
      upload(gl, this.data, this.w, this.dirtyY0, this.dirtyY1);
    }
    this.dirtyY0 = Infinity;
    this.dirtyY1 = 0;
  }
}

/** `white-space: nowrap`: runs of spaces, tabs and newlines as one space, none at the ends. */
export function collapseSpaces(text: string): string {
  return /[\t\n\r\f]| {2}|^ | $/.test(text) ? text.replace(/[ \t\n\r\f]+/g, " ").trim() : text;
}

/** End offsets of the grapheme clusters of `text` (code points without `Intl.Segmenter`). */
function clusterEnds(text: string): number[] {
  const ends: number[] = [];
  const Seg = (
    Intl as unknown as { Segmenter?: new (l?: string, o?: { granularity: string }) => { segment(s: string): Iterable<{ index: number; segment: string }> } }
  ).Segmenter;
  if (Seg) {
    for (const s of new Seg(undefined, { granularity: "grapheme" }).segment(text)) ends.push(s.index + s.segment.length);
  } else {
    let i = 0;
    for (const ch of text) ends.push((i += ch.length));
  }
  return ends;
}

function allocate(gl: GL, w: number, h: number): void {
  gl.texImage2D(gl.TEXTURE_2D, 0, gl.R8, w, h, 0, gl.RED, gl.UNSIGNED_BYTE, null);
  // Texels are fetched (texelFetch): no filtering, but a complete texture.
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
}

function upload(gl: GL, data: Uint8Array, w: number, y0: number, y1: number): void {
  gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
  gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, y0, w, y1 - y0, gl.RED, gl.UNSIGNED_BYTE, data.subarray(y0 * w, y1 * w));
}
