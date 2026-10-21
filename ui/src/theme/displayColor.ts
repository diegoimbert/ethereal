/**
 * Track / clip / marker colors as shown: a color from outside the app's palette (imported from
 * another DAW: a very dark blue, a saturated red) is brought into the palette's range, keeping
 * its hue, so it reads like the others on the app's surfaces. The palette's own colors (and any
 * color already in that range) are shown as is. The document keeps the color it has.
 */

/** OKLCH lightness and chroma range of `TRACK_COLORS` (soft, luminous hues). */
const L_MIN = 0.72;
const L_MAX = 0.89;
const C_MAX = 0.12;

type Vec3 = [number, number, number];

const toLinear = (c: number) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
const toGamma = (c: number) => (c <= 0.0031308 ? 12.92 * c : 1.055 * c ** (1 / 2.4) - 0.055);

function rgbToOklab([r, g, b]: Vec3): Vec3 {
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  return [
    0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
    1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
    0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
  ];
}

function oklabToRgb([L, a, b]: Vec3): Vec3 {
  const l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s = (L - 0.0894841775 * a - 1.291485548 * b) ** 3;
  return [
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  ];
}

const inGamut = (rgb: Vec3) => rgb.every((c) => c >= -1e-4 && c <= 1 + 1e-4);

const cache = new Map<number, number>();

/** `color` (0xRRGGBB) as shown: within the palette's lightness and chroma range. */
export function displayColor(color: number): number {
  const key = color & 0xffffff;
  const hit = cache.get(key);
  if (hit !== undefined) return hit;
  const rgb: Vec3 = [toLinear(((key >> 16) & 0xff) / 255), toLinear(((key >> 8) & 0xff) / 255), toLinear((key & 0xff) / 255)];
  const [L, a, b] = rgbToOklab(rgb);
  const C = Math.hypot(a, b);
  let out = key;
  if (L < L_MIN || L > L_MAX || C > C_MAX) {
    const l = Math.min(L_MAX, Math.max(L_MIN, L));
    const h = Math.atan2(b, a);
    // The hue at the clamped chroma; less chroma while it falls outside sRGB.
    let c = Math.min(C, C_MAX);
    let lin = oklabToRgb([l, c * Math.cos(h), c * Math.sin(h)]);
    while (!inGamut(lin) && c > 0) {
      c = Math.max(0, c - 0.005);
      lin = oklabToRgb([l, c * Math.cos(h), c * Math.sin(h)]);
    }
    const ch = (v: number) => Math.round(Math.min(1, Math.max(0, toGamma(Math.min(1, Math.max(0, v))))) * 255);
    out = (ch(lin[0]) << 16) | (ch(lin[1]) << 8) | ch(lin[2]);
  }
  cache.set(key, out);
  return out;
}

/** `displayColor` as a CSS hex color. */
export function displayColorCss(color: number): string {
  return `#${displayColor(color).toString(16).padStart(6, "0")}`;
}
