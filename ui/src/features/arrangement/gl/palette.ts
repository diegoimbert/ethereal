/**
 * Colors of the WebGL arranger, read from the page's CSS custom properties (so a theme
 * switch, a restyled `tokens.css` or a devtools override applies), and the clip color
 * mixes `arrangement.css` does with `color-mix(in srgb, …)`, computed on the CPU.
 *
 * Colors are `[r, g, b, a]` floats in 0–1 (straight alpha).
 */

import { displayColor, getGlow, getTheme, readToken, themeColorValue, tokenName, type ColorKey, type ThemeName } from "@/theme";

export type Rgba = readonly [number, number, number, number];

let normalizer: CanvasRenderingContext2D | null | undefined;

/** Any CSS color string as RGBA floats (named colors, hex, rgb(), …). */
export function parseColor(css: string, fallback: Rgba = [0, 0, 0, 1]): Rgba {
  const s = css.trim();
  const hex = /^#([0-9a-f]{3,8})$/i.exec(s);
  if (hex) return parseHex(hex[1]!) ?? fallback;
  const rgb = /^rgba?\(\s*([\d.]+)[\s,]+([\d.]+)[\s,]+([\d.]+)(?:[\s,/]+([\d.]+%?))?\s*\)$/i.exec(s);
  if (rgb) {
    const a = rgb[4] === undefined ? 1 : rgb[4].endsWith("%") ? parseFloat(rgb[4]) / 100 : parseFloat(rgb[4]);
    return [+rgb[1]! / 255, +rgb[2]! / 255, +rgb[3]! / 255, a];
  }
  if (normalizer === undefined) normalizer = typeof document === "undefined" ? null : document.createElement("canvas").getContext("2d");
  if (!normalizer || !s) return fallback;
  normalizer.fillStyle = "#000"; // eth-allow-hardcoded: reset before parsing
  normalizer.fillStyle = s;
  const out = normalizer.fillStyle;
  return out === s ? fallback : parseColor(out, fallback);
}

function parseHex(h: string): Rgba | null {
  const full = h.length <= 4 ? [...h].map((c) => c + c).join("") : h;
  if (full.length !== 6 && full.length !== 8) return null;
  const n = (i: number) => parseInt(full.slice(i, i + 2), 16) / 255;
  return [n(0), n(2), n(4), full.length === 8 ? n(6) : 1];
}

/** A 0xRRGGBB clip/track color, as shown (`displayColor`: an imported color in the palette's range). */
export function rgbOf(color: number): Rgba {
  const c = displayColor(color);
  return [((c >> 16) & 0xff) / 255, ((c >> 8) & 0xff) / 255, (c & 0xff) / 255, 1];
}

/** `color-mix(in srgb, a p, b)`: `p` of `a` (0–1), alpha mixed too. */
export function mix(a: Rgba, p: number, b: Rgba): Rgba {
  const q = 1 - p;
  const alpha = a[3] * p + b[3] * q;
  if (alpha <= 0) return [0, 0, 0, 0];
  // color-mix mixes premultiplied components, then un-premultiplies.
  const c = (i: number) => (a[i]! * a[3] * p + b[i]! * b[3] * q) / alpha;
  return [c(0), c(1), c(2), alpha];
}

/** `color` with alpha `a` (`color-mix(in srgb, color a, transparent)`). */
export function withAlpha(color: Rgba, a: number): Rgba {
  return [color[0], color[1], color[2], color[3] * a];
}

export interface Palette {
  theme: ThemeName;
  /** Selection halos (Settings > Appearance > Glow); only `false` turns them off. */
  glow?: boolean;
  /** Arranger background (`--eth-color-bg`): muted clips are mixed into it. */
  bg: Rgba;
  /** Clip bodies mix the clip color into it (`--eth-color-bg-panel`). */
  panel: Rgba;
  /** Clip title text (`--eth-color-text-inverse`). */
  titleText: Rgba;
  /** Loop seams in clip bodies (`--eth-color-clip-seam`). */
  seam: Rgba;
  /** Waveform peaks past full scale (`--eth-color-meter-high`). */
  over: Rgba;
  /** Edge handle hover (`--eth-color-selection`). */
  selection: Rgba;
  /** `--eth-mix-white` / `--eth-mix-black`. */
  white: Rgba;
  black: Rgba;
  /** A loading track's clips (#260): their opacity (`--eth-opacity-muted`), default 0.6. */
  loadingOpacity?: number;
  /** How long a loaded track's clips take to fade back (`--eth-duration-slow`, ms). */
  loadedFadeMs?: number;
}

/** A number token (`0.6`, `300ms`, `0.3s`), or `fallback`. */
function numberToken(name: `--${string}`, fallback: number): number {
  const v = readToken(name).trim();
  const n = parseFloat(v);
  if (!Number.isFinite(n)) return fallback;
  return v.endsWith("ms") ? n : v.endsWith("s") ? n * 1000 : n;
}

type PaletteKey = Extract<ColorKey, "bg" | "bgPanel" | "textInverse" | "clipSeam" | "meterHigh" | "selection">;

/** The palette of the current theme, read from the page. */
export function readPalette(): Palette {
  const theme = getTheme();
  const color = (key: PaletteKey): Rgba => {
    const css = readToken(tokenName("color", key) as `--${string}`) || themeColorValue(key);
    return parseColor(css);
  };
  return {
    theme,
    glow: getGlow(),
    bg: color("bg"),
    panel: color("bgPanel"),
    titleText: color("textInverse"),
    seam: color("clipSeam"),
    over: color("meterHigh"),
    selection: color("selection"),
    white: parseColor(readToken("--eth-mix-white"), [1, 1, 1, 1]),
    black: parseColor(readToken("--eth-mix-black"), [0, 0, 0, 1]),
    loadingOpacity: numberToken("--eth-opacity-muted", 0.6),
    loadedFadeMs: numberToken("--eth-duration-slow", 300),
  };
}

/** The colors a clip is drawn with (see `.eth-clip` in arrangement.css). */
export interface ClipColors {
  /** `--clip-strong`: title band, waveform and notes. */
  strong: Rgba;
  /** Clip box background (`color-mix(strong 26%, panel)`). */
  body: Rgba;
  /** Selected box background (`color-mix(strong 36%, bg)`). */
  bodySelected: Rgba;
  /** Selected title band (`color-mix(strong 70%, lift)`). */
  titleSelected: Rgba;
  /** Hovered title band (`color-mix(strong 85%, lift)`). */
  titleHover: Rgba;
  /** Selection rim (`--clip-rim`). */
  rim: Rgba;
  /** Selection halo layers (none in the light theme). */
  glowInner: Rgba | null;
  glowOuter: Rgba | null;
}

/** Clip colors for a clip color, cached per palette. */
export function clipColors(p: Palette, color: number): ClipColors {
  let cache = clipColorCache.get(p);
  if (!cache) clipColorCache.set(p, (cache = new Map()));
  let c = cache.get(color);
  if (c) return c;
  const base = rgbOf(color);
  const light = p.theme === "light";
  const strong = light ? mix(base, 0.7, p.black) : base;
  const lift = light ? p.black : p.white;
  c = {
    strong,
    body: mix(strong, 0.26, p.panel),
    bodySelected: mix(strong, 0.36, p.bg),
    titleSelected: mix(strong, 0.7, lift),
    titleHover: mix(strong, 0.85, lift),
    rim: light ? strong : mix(base, 0.3, p.white),
    glowInner: light || p.glow === false ? null : withAlpha(strong, 0.85),
    glowOuter: light || p.glow === false ? null : withAlpha(strong, 0.45),
  };
  cache.set(color, c);
  return c;
}

/**
 * A compact lane's clip name (drawn in the clip's color on its body): brightened like the
 * border when the clip is selected (the selection rim's color), so both read as selected.
 */
export function compactNameColor(c: ClipColors, selected: boolean): Rgba {
  return selected ? c.rim : c.strong;
}

const clipColorCache = new WeakMap<Palette, Map<number, ClipColors>>();
