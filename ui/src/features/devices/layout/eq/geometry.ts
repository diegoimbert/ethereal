/**
 * Pure geometry of the `EqCurve` widget: the log-frequency / dB plot mapping, band
 * resolution from the widget's param bindings, handle placement and hit testing.
 */

import type { EqBandBinding, EqShape } from "@/generated";
import { labelIndex } from "../../paramScale";
import type { ParamBinding } from "../context";
import { F_MAX, F_MIN, freqToX, xToFreq } from "../curves";
import type { EqBand } from "./eqResponse";

/** Visible gain range of the curve (±dB; the EQ's band gains span the same). */
export const DB_RANGE = 24;
/** dB grid lines (labels on the multiples of 12). */
export const DB_TICKS = [-18, -12, -6, 0, 6, 12, 18];
/** Frequency grid lines; `FREQ_LABELS` get a caption. */
export const FREQ_TICKS = [30, 40, 50, 60, 70, 80, 90, 100, 200, 300, 400, 500, 600, 700, 800, 900, 1000, 2000, 3000, 4000, 5000, 6000, 7000, 8000, 9000, 10000];
export const FREQ_LABELS = [50, 100, 200, 500, 1000, 2000, 5000, 10000];
/** Spectrum overlay scale (dBFS at the bottom / top of the plot). */
export const SPECTRUM_FLOOR_DB = -90;
export const SPECTRUM_TOP_DB = 6;
/** Inset (px) keeping handles at the gain extremes fully visible. */
export const PAD = 10;
/** Handle hit radius (px). */
export const HIT = 14;
/** Q when a band has no Q param (`EqBandBinding.q = None`). */
export const DEFAULT_Q = Math.SQRT1_2;

export { F_MAX, F_MIN };

export interface Plot {
  w: number;
  h: number;
}

/** Frequency (Hz) → x (px). */
export const xOf = (p: Plot, hz: number) => freqToX(hz) * p.w;
/** x (px) → frequency (Hz). */
export const freqAt = (p: Plot, x: number) => xToFreq(x / Math.max(1, p.w));
/** Curve dB → y (px), clamped to the plot. */
export function yOf(p: Plot, db: number): number {
  const t = (Math.max(-DB_RANGE, Math.min(DB_RANGE, db)) + DB_RANGE) / (2 * DB_RANGE);
  return PAD + (1 - t) * Math.max(0, p.h - 2 * PAD);
}
/** Pixels per dB on the curve scale. */
export const pxPerDb = (p: Plot) => Math.max(1, p.h - 2 * PAD) / (2 * DB_RANGE);
/** Spectrum dBFS → y (px). */
export function spectrumY(p: Plot, db: number): number {
  const t = (db - SPECTRUM_FLOOR_DB) / (SPECTRUM_TOP_DB - SPECTRUM_FLOOR_DB);
  return (1 - Math.max(0, Math.min(1, t))) * p.h;
}

/** Shapes whose gain param changes the response (the others ignore vertical drags). */
export function usesGain(shape: EqShape): boolean {
  return shape === "Bell" || shape === "LowShelf" || shape === "HighShelf";
}

/** One band of the widget with its live bindings. */
export interface ResolvedBand extends EqBand {
  index: number;
  bindings: {
    on: ParamBinding | null;
    kind: ParamBinding | null;
    freq: ParamBinding;
    gain: ParamBinding | null;
    q: ParamBinding | null;
  };
}

/** Resolve one `EqBandBinding` against its param bindings (`null` = a missing param). */
export function resolveBand(
  index: number,
  spec: EqBandBinding,
  bind: (id: number | null) => ParamBinding | null,
): ResolvedBand | null {
  const freq = bind(spec.freq);
  if (!freq) return null;
  const on = bind(spec.on);
  const kind = bind(spec.kind);
  const gain = bind(spec.gain);
  const q = bind(spec.q);
  const shapeIndex = kind ? labelIndex(kind.info, kind.plain) : 0;
  const shape = spec.shapes[shapeIndex] ?? spec.shapes[0] ?? "Bell";
  return {
    index,
    shape,
    freq: freq.plain,
    gainDb: gain ? gain.plain : 0,
    q: q ? q.plain : DEFAULT_Q,
    on: on ? on.plain >= 0.5 : true,
    bindings: { on, kind, freq, gain, q },
  };
}

/** Handle position of a band: its frequency, at its gain (0 dB for gainless shapes). */
export function handleAt(p: Plot, b: EqBand): { x: number; y: number } {
  return { x: xOf(p, b.freq), y: yOf(p, usesGain(b.shape) ? b.gainDb : 0) };
}

/**
 * The band whose handle is nearest `(x, y)` within `HIT` px, or `null`. Ties prefer the
 * selected band, then enabled bands (a disabled handle under an enabled one stays reachable
 * through the other).
 */
export function hitBand(p: Plot, bands: ReadonlyArray<EqBand & { index: number }>, x: number, y: number, selected: number | null): number | null {
  let best: number | null = null;
  let bestScore = Infinity;
  for (const b of bands) {
    const h = handleAt(p, b);
    const d = Math.hypot(h.x - x, h.y - y);
    if (d > HIT) continue;
    const score = d - (b.index === selected ? 3 : 0) - (b.on ? 1 : 0);
    if (score < bestScore) {
      bestScore = score;
      best = b.index;
    }
  }
  return best;
}

/** `n` log-spaced frequencies from `F_MIN` to `F_MAX` (the curve's sample points). */
export function logFreqs(n: number): number[] {
  const out: number[] = [];
  for (let i = 0; i < n; i++) out.push(F_MIN * Math.pow(F_MAX / F_MIN, i / Math.max(1, n - 1)));
  return out;
}

/** Frequency caption ("50", "1k", "10k"). */
export function freqLabel(hz: number): string {
  return hz >= 1000 ? `${hz / 1000}k` : `${hz}`;
}

/** New Q after a Q drag of `dy` px (up = narrower) or a wheel step. */
export function qAfter(q0: number, dyPx: number): number {
  return q0 * Math.pow(2, -dyPx / 48);
}
