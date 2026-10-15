/**
 * The browser's own text rasterizer for the WebGL arranger: Canvas2D `fillText` in the font
 * the DOM uses for clip names, so sprites carry the browser's shaping (kerning, ligatures,
 * contextual alternates), hinting, anti-aliasing and gamma, exactly as the DOM
 * shows them.
 *
 * What makes it match the DOM pixel for pixel (measured in Chrome, `webgl-arranger-text`):
 *
 * - **Font features.** The app sets `font-feature-settings: "cv11"` and
 *   `font-variant-numeric: tabular-nums`, which a canvas can't. The UI font's `@font-face`
 *   rules are registered again under an alias with the features as a descriptor
 *   (`FontFace.featureSettings`), and the canvas draws in the alias.
 * - **Whole-run shaping.** Chrome's canvas shapes word by word unless
 *   `textRendering = "optimizeLegibility"`, which loses contextual forms across spaces
 *   (Inter raises "—" between digits and capitals): set it.
 * - **The ink's gamma.** Skia adjusts glyph coverage for the text color's luminance
 *   (lighter for light ink, heavier for dark). The mask is rasterized in the ink's color
 *   reduced to 3 bits per channel (`canonicalInk`, Skia's own quantization), and only its
 *   alpha is kept: a dark-ink mask differs from a white one by up to 23/255.
 */

import type { Rgba } from "./palette";

export interface FontSpec {
  /** CSS px. */
  size: number;
  weight: number;
}

/** A rasterized run: alpha coverage, `width` × `height` device px, the pen at (`originX`, `baseline`). */
export interface RasterRun {
  data: Uint8Array;
  width: number;
  height: number;
  originX: number;
  baseline: number;
  /** Advance (css px). */
  advance: number;
}

/** What the atlas needs from the browser (a fake in unit tests). */
export interface TextRasterizer {
  /** Re-reads the text style (font loaded, theme changed); `dpr` = device pixel ratio. */
  reset(dpr: number): void;
  /** Advance of `text` (css px). */
  measure(text: string, font: FontSpec): number;
  /** The font's ascent and descent (css px). */
  metrics(font: FontSpec): { ascent: number; descent: number };
  /** `text` at device pixels, in `ink` (a `canonicalInk` key: `#rrggbb`). */
  draw(text: string, font: FontSpec, ink: string): RasterRun | null;
  /** A stroked icon (SVG paths in a 24×24 box), `px` device px square: coverage. */
  icon(paths: ReadonlyArray<string>, px: number, strokeWidth: number): Uint8Array;
}

/**
 * The color Skia quantizes text ink to before picking its coverage adjustment (3 bits per
 * channel), as `#rrggbb`: inks with the same key rasterize to the same mask.
 */
export function canonicalInk(ink: Rgba): string {
  let s = "#";
  for (let k = 0; k < 3; k++) {
    const v = Math.round(Math.min(1, Math.max(0, ink[k]!)) * 255) & 0xe0;
    s += v.toString(16).padStart(2, "0");
  }
  return s;
}

/** Canvas `letterSpacing` for `letter-spacing: normal`. */
const NO_SPACING = "0px"; // eth-allow-hardcoded: the canvas API value, not a design value

/** The text style of the DOM's clip names, from the computed styles. */
interface TextStyle {
  family: string;
  /** Feature tags (`"cv11"`, `"tnum"`), as a `font-feature-settings` value; "" for none. */
  features: string;
  letterSpacing: string;
}

/** `font-variant-numeric` keywords as OpenType features. */
const NUMERIC_FEATURES: Record<string, string> = {
  "lining-nums": "lnum",
  "oldstyle-nums": "onum",
  "proportional-nums": "pnum",
  "tabular-nums": "tnum",
  "diagonal-fractions": "frac",
  "stacked-fractions": "afrc",
  ordinal: "ordn",
  "slashed-zero": "zero",
};

/** Reads the clip name's style (`.eth-clip__name` under the theme root). */
function readTextStyle(): TextStyle {
  const fallback: TextStyle = { family: "sans-serif", features: "", letterSpacing: NO_SPACING };
  if (typeof document === "undefined" || !document.body) return fallback;
  const probe = document.createElement("span");
  probe.className = "eth-clip__name";
  probe.style.cssText = "position:absolute;visibility:hidden;pointer-events:none";
  const host = document.querySelector("[data-theme]") ?? document.body;
  host.appendChild(probe);
  const cs = getComputedStyle(probe);
  const tags: string[] = [];
  const ffs = cs.fontFeatureSettings;
  if (ffs && ffs !== "normal") tags.push(...ffs.split(",").map((t) => t.trim()));
  for (const kw of (cs.fontVariantNumeric || "normal").split(/\s+/)) {
    const tag = NUMERIC_FEATURES[kw];
    if (tag) tags.push(`"${tag}"`);
  }
  const style: TextStyle = {
    family: cs.fontFamily || fallback.family,
    features: tags.join(", "),
    letterSpacing: cs.letterSpacing && cs.letterSpacing !== "normal" ? cs.letterSpacing : NO_SPACING,
  };
  probe.remove();
  return style;
}

/** Aliases registered (family + features → canvas family list), once per page. */
const aliases = new Map<string, string>();

/** The first family of a CSS `font-family` list, unquoted, and the rest of the list. */
function splitFamily(list: string): [string, string] {
  const m = /^\s*(?:"([^"]+)"|'([^']+)'|([^,]+))\s*(?:,(.*))?$/.exec(list);
  if (!m) return [list.trim(), ""];
  return [(m[1] ?? m[2] ?? m[3] ?? "").trim(), (m[4] ?? "").trim()];
}

/** `url(...)` sources of a `src` descriptor, resolved against the stylesheet. */
function absoluteSrc(src: string, base: string): string {
  return src.replace(/url\(\s*(['"]?)([^'")]+)\1\s*\)/g, (_, _q: string, u: string) => {
    try {
      return `url("${new URL(u, base).href}")`;
    } catch {
      return `url("${u}")`;
    }
  });
}

/**
 * A canvas font-family list drawing like `style` with its features: the first family's
 * `@font-face` rules registered again under an alias with the features as a descriptor.
 * Without features (or `@font-face` rules to copy), the family list as is.
 */
function canvasFamily(style: TextStyle): string {
  if (!style.features || typeof document === "undefined" || typeof FontFace === "undefined") return style.family;
  const key = `${style.family}|${style.features}`;
  const hit = aliases.get(key);
  if (hit !== undefined) return hit;
  const [first, rest] = splitFamily(style.family);
  const alias = `eth-text-${aliases.size}`;
  let faces = 0;
  for (const sheet of Array.from(document.styleSheets)) {
    let rules: CSSRuleList;
    try {
      rules = sheet.cssRules;
    } catch {
      continue; // cross-origin
    }
    for (const rule of Array.from(rules)) {
      if (!(rule instanceof CSSFontFaceRule)) continue;
      const s = rule.style;
      if (splitFamily(s.getPropertyValue("font-family"))[0] !== first) continue;
      const src = s.getPropertyValue("src");
      if (!src) continue;
      try {
        const descriptors: FontFaceDescriptors & { featureSettings?: string } = { featureSettings: style.features };
        const weight = s.getPropertyValue("font-weight");
        const fstyle = s.getPropertyValue("font-style");
        const range = s.getPropertyValue("unicode-range");
        const stretch = s.getPropertyValue("font-stretch");
        if (weight) descriptors.weight = weight;
        if (fstyle) descriptors.style = fstyle;
        if (range) descriptors.unicodeRange = range;
        if (stretch) descriptors.stretch = stretch;
        document.fonts.add(new FontFace(alias, absoluteSrc(src, sheet.href ?? document.baseURI), descriptors));
        faces++;
      } catch {
        // an invalid descriptor: skip the face
      }
    }
  }
  const family = faces ? `"${alias}"${rest ? `, ${rest}` : ""}` : style.family;
  aliases.set(key, family);
  return family;
}

type Ctx2D = CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D;

/** Canvas2D `fillText` in the DOM's clip-name style. */
export class CanvasTextRasterizer implements TextRasterizer {
  private family = "sans-serif";
  private letterSpacing = NO_SPACING;
  private dpr = 1;
  private canvas: HTMLCanvasElement | OffscreenCanvas | null = null;
  private ctx: Ctx2D | null = null;
  private fontsRequested = new Set<string>();
  /** The context's current font (`""`: its state was reset). */
  private current = "";
  private metricCache = new Map<string, { ascent: number; descent: number }>();

  reset(dpr: number): void {
    this.dpr = dpr;
    const style = readTextStyle();
    this.family = canvasFamily(style);
    this.letterSpacing = style.letterSpacing;
    this.metricCache.clear();
    this.fontsRequested.clear();
    this.current = "";
  }

  private context(w = 1, h = 1): Ctx2D | null {
    if (!this.ctx) {
      if (typeof OffscreenCanvas !== "undefined") this.canvas = new OffscreenCanvas(64, 32);
      else if (typeof document !== "undefined") this.canvas = document.createElement("canvas");
      else return null;
      this.ctx = (this.canvas.getContext("2d", { willReadFrequently: true }) as Ctx2D | null) ?? null;
      if (!this.ctx) return null;
    }
    const c = this.canvas!;
    if (c.width < w || c.height < h) {
      // Resizing resets the context's state: `font()` sets it again.
      c.width = Math.max(c.width, Math.ceil(w / 64) * 64);
      c.height = Math.max(c.height, Math.ceil(h / 16) * 16);
      this.current = "";
    }
    return this.ctx;
  }

  private font(ctx: Ctx2D, font: FontSpec): void {
    const css = `${font.weight} ${font.size}px ${this.family}`;
    if (css === this.current) return;
    this.current = css;
    ctx.font = css;
    const ext = ctx as Ctx2D & { letterSpacing?: string; textRendering?: string; fontKerning?: string };
    if ("letterSpacing" in ext) ext.letterSpacing = this.letterSpacing;
    // Shape the whole run (Chrome's canvas otherwise shapes word by word).
    if ("textRendering" in ext) ext.textRendering = "optimizeLegibility";
    ctx.textBaseline = "alphabetic";
    ctx.textAlign = "left";
    // The alias's faces load on first use; `document.fonts` then fires `loadingdone`, which
    // resets the atlas (GlClipLayer).
    if (!this.fontsRequested.has(css) && typeof document !== "undefined" && document.fonts) {
      this.fontsRequested.add(css);
      void document.fonts.load(css).catch(() => undefined);
    }
  }

  measure(text: string, font: FontSpec): number {
    const ctx = this.context();
    if (!ctx) return text.length * font.size * 0.55;
    this.font(ctx, font);
    return ctx.measureText(text).width;
  }

  metrics(font: FontSpec): { ascent: number; descent: number } {
    const key = `${font.weight}|${font.size}`;
    const hit = this.metricCache.get(key);
    if (hit) return hit;
    let m = { ascent: font.size * 0.97, descent: font.size * 0.24 };
    const ctx = this.context();
    if (ctx) {
      this.font(ctx, font);
      const t = ctx.measureText("Hg");
      if (t.fontBoundingBoxAscent) m = { ascent: t.fontBoundingBoxAscent, descent: t.fontBoundingBoxDescent };
    }
    this.metricCache.set(key, m);
    return m;
  }

  draw(text: string, font: FontSpec, ink: string): RasterRun | null {
    let ctx = this.context();
    if (!ctx) return null;
    this.font(ctx, font);
    const m = ctx.measureText(text);
    const dpr = this.dpr;
    const metrics = this.metrics(font);
    // The ink's box at device px, one texel of margin, the pen on a device pixel.
    const originX = Math.max(0, Math.ceil(m.actualBoundingBoxLeft * dpr)) + 1;
    const right = Math.max(0, Math.ceil(m.actualBoundingBoxRight * dpr)) + 1;
    const baseline = Math.ceil(Math.max(m.actualBoundingBoxAscent, metrics.ascent) * dpr) + 1;
    const below = Math.ceil(Math.max(m.actualBoundingBoxDescent, metrics.descent) * dpr) + 1;
    const width = originX + right;
    const height = baseline + below;
    if (width <= 2 || height <= 2) return null;
    ctx = this.context(width, height)!;
    this.font(ctx, font);
    // Faces of other scripts (unicode-range subsets) load on demand: `loadingdone` then
    // resets the atlas and this text is drawn again in them.
    if (/[^ -\u00ff]/.test(text) && typeof document !== "undefined" && document.fonts) void document.fonts.load(this.current, text).catch(() => undefined);
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, width, height);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.fillStyle = ink;
    ctx.fillText(text, originX / dpr, baseline / dpr);
    const img = ctx.getImageData(0, 0, width, height).data;
    const data = new Uint8Array(width * height);
    for (let i = 0; i < data.length; i++) data[i] = img[i * 4 + 3]!;
    return { data, width, height, originX, baseline, advance: m.width };
  }

  icon(paths: ReadonlyArray<string>, px: number, strokeWidth: number): Uint8Array {
    const mask = new Uint8Array(px * px);
    const ctx = this.context(px, px);
    if (!ctx || typeof Path2D === "undefined") return mask;
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, px, px);
    ctx.setTransform(px / 24, 0, 0, px / 24, 0, 0);
    ctx.lineWidth = strokeWidth;
    ctx.lineCap = "round";
    ctx.lineJoin = "round";
    ctx.strokeStyle = "#fff"; // eth-allow-hardcoded: a mask, tinted by the shader
    for (const d of paths) ctx.stroke(new Path2D(d));
    const img = ctx.getImageData(0, 0, px, px).data;
    for (let i = 0; i < mask.length; i++) mask[i] = img[i * 4 + 3]!;
    return mask;
  }
}
