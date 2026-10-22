// eth-allow-case-file: color math on token values.
/**
 * Color math for themes (pure, no DOM): parsing the color syntaxes the tokens use, mixing
 * (`color-mix(in srgb, …)`), and WCAG contrast. Used to derive a theme's tokens from its main
 * colors (`themeModel.ts`), to validate imported themes and by the contrast checks.
 */

/** A color as 0–255 channels and a 0–1 alpha. */
export interface Rgba {
  r: number;
  g: number;
  b: number;
  a: number;
}

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

/**
 * Parses `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, `rgb()`/`rgba()` (comma or space syntax,
 * `/ alpha`, alpha as a number or a %) and `transparent`. Anything else: `null`.
 */
export function parseColor(css: string): Rgba | null {
  const s = css.trim().toLowerCase();
  if (s === "transparent") return { r: 0, g: 0, b: 0, a: 0 };
  const hex = /^#([0-9a-f]{3,8})$/.exec(s);
  if (hex) {
    const h = hex[1]!;
    if (h.length === 5 || h.length === 7) return null;
    const full = h.length <= 4 ? [...h].map((c) => c + c).join("") : h;
    const n = (i: number) => parseInt(full.slice(i, i + 2), 16);
    return { r: n(0), g: n(2), b: n(4), a: full.length === 8 ? n(6) / 255 : 1 };
  }
  const fn = /^rgba?\(\s*([\d.]+)\s*[,\s]\s*([\d.]+)\s*[,\s]\s*([\d.]+)\s*(?:[,/]\s*([\d.]+%?)\s*)?\)$/.exec(s);
  if (fn) {
    const [r, g, b] = [fn[1], fn[2], fn[3]].map(Number) as [number, number, number];
    if ([r, g, b].some((v) => !Number.isFinite(v) || v > 255)) return null;
    const alpha = fn[4] === undefined ? 1 : fn[4].endsWith("%") ? parseFloat(fn[4]) / 100 : parseFloat(fn[4]);
    if (!Number.isFinite(alpha) || alpha > 1) return null;
    return { r, g, b, a: alpha };
  }
  return null;
}

/** Whether `css` is a color this module understands (what a theme file may use). */
export function isColor(css: unknown): css is string {
  return typeof css === "string" && parseColor(css) !== null;
}

const hex2 = (v: number) =>
  Math.round(clamp(v, 0, 255))
    .toString(16)
    .padStart(2, "0");

/** `#rrggbb` (opaque) or `rgba(r, g, b, a)`. */
export function formatColor(c: Rgba): string {
  if (c.a >= 1) return `#${hex2(c.r)}${hex2(c.g)}${hex2(c.b)}`;
  const a = Math.round(clamp(c.a, 0, 1) * 1000) / 1000;
  return `rgba(${Math.round(c.r)}, ${Math.round(c.g)}, ${Math.round(c.b)}, ${a})`;
}

/** `#rrggbb` of `css` (alpha dropped), for `<input type="color">`; `null` if not a color. */
export function toHex(css: string): string | null {
  const c = parseColor(css);
  return c ? formatColor({ ...c, a: 1 }) : null;
}

function req(css: string): Rgba {
  const c = parseColor(css);
  if (!c) throw new Error(`not a color: ${css}`);
  return c;
}

/** `color-mix(in srgb, a p, b)` with `p` (0–1) of `a`, as a hex color (both opaque). */
export function mix(a: string, p: number, b: string): string {
  const x = req(a);
  const y = req(b);
  const q = 1 - p;
  return formatColor({ r: x.r * p + y.r * q, g: x.g * p + y.g * q, b: x.b * p + y.b * q, a: 1 });
}

/** `css` with alpha `a`. */
export function withAlpha(css: string, a: number): string {
  return formatColor({ ...req(css), a });
}

/** `fg` composited over the opaque `bg`. */
export function over(fg: string, bg: string): Rgba {
  const f = req(fg);
  const b = req(bg);
  return { r: f.r * f.a + b.r * (1 - f.a), g: f.g * f.a + b.g * (1 - f.a), b: f.b * f.a + b.b * (1 - f.a), a: 1 };
}

const channel = (v: number) => {
  const s = v / 255;
  return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
};

/** WCAG relative luminance of an opaque color. */
export function luminance(c: Rgba): number {
  return 0.2126 * channel(c.r) + 0.7152 * channel(c.g) + 0.0722 * channel(c.b);
}

/** WCAG contrast ratio (1–21) of `fg` on `bg`; a translucent `fg` is composited over `bg`. */
export function contrast(fg: string, bg: string): number {
  const b = over(bg, "#000000");
  const f = over(fg, formatColor(b));
  const [l1, l2] = [luminance(f), luminance(b)].sort((x, y) => y - x) as [number, number];
  return (l1 + 0.05) / (l2 + 0.05);
}

/** `css` with its HSL saturation raised by `ds` and lightness by `dl` (both −1–1). */
export function adjustHsl(css: string, ds: number, dl: number): string {
  const { r, g, b, a } = req(css);
  const [rn, gn, bn] = [r / 255, g / 255, b / 255];
  const max = Math.max(rn, gn, bn);
  const min = Math.min(rn, gn, bn);
  let h = 0;
  let s = 0;
  const l = (max + min) / 2;
  const d = max - min;
  if (d > 0) {
    s = d / (1 - Math.abs(2 * l - 1));
    if (max === rn) h = ((gn - bn) / d) % 6;
    else if (max === gn) h = (bn - rn) / d + 2;
    else h = (rn - gn) / d + 4;
    h *= 60;
  }
  const s2 = clamp(s + ds, 0, 1);
  const l2 = clamp(l + dl, 0, 1);
  const c = (1 - Math.abs(2 * l2 - 1)) * s2;
  const x = c * (1 - Math.abs(((h / 60) % 2) - 1));
  const m = l2 - c / 2;
  const hp = (h + 360) % 360;
  const [r1, g1, b1] = hp < 60 ? [c, x, 0] : hp < 120 ? [x, c, 0] : hp < 180 ? [0, c, x] : hp < 240 ? [0, x, c] : hp < 300 ? [x, 0, c] : [c, 0, x];
  return formatColor({ r: (r1 + m) * 255, g: (g1 + m) * 255, b: (b1 + m) * 255, a });
}
