/**
 * Pure geometry of the graphic EQ editor: the log-frequency axis (10 Hz..30 kHz), the dB
 * axis (± the display range), band resolution from the widget's bindings, handle placement
 * and hit testing.
 */

import type { GraphicEqBandBinding, GraphicEqShape } from "@/generated";
import type { ParamBinding } from "../context";
import { SHAPES, usesGain, type GraphicBand } from "./response";

/** Frequency axis (Hz): 0 Hz can't sit on a log axis, so it starts at 10 Hz. */
export const F_LO = 10;
export const F_HI = 30000;
/** Display ranges (± dB) the range button cycles through; the default. */
export const RANGES: ReadonlyArray<number> = [6, 12, 24, 30];
export const DEFAULT_RANGE = 12;
/** Inset (px) keeping handles at the extremes visible. */
export const PAD = 12;
/** Handle hit radius (px). */
export const HIT = 12;
/** Frequency grid lines; `FREQ_LABELS` get a caption. */
export const FREQ_TICKS = [
  20, 30, 40, 50, 60, 70, 80, 90, 100, 200, 300, 400, 500, 600, 700, 800, 900, 1000, 2000, 3000, 4000, 5000, 6000, 7000, 8000, 9000, 10000, 20000,
];
export const FREQ_LABELS = [20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000];

export interface Box {
  w: number;
  h: number;
}

const LOG_SPAN = Math.log(F_HI / F_LO);
const clamp = (v: number, lo: number, hi: number) => (v < lo ? lo : v > hi ? hi : v);

/** Frequency → position on the axis (0..1). */
export const freqToT = (hz: number) => Math.log(Math.max(hz, 1e-9) / F_LO) / LOG_SPAN;
/** Position on the axis (0..1) → frequency. */
export const tToFreq = (t: number) => F_LO * Math.exp(clamp(t, 0, 1) * LOG_SPAN);
export const xOf = (b: Box, hz: number) => freqToT(hz) * b.w;
export const freqAt = (b: Box, x: number) => tToFreq(x / Math.max(1, b.w));

/** dB → y (px), clamped to the plot. */
export function yOf(b: Box, db: number, range: number): number {
  const t = (clamp(db, -range, range) + range) / (2 * range);
  return PAD + (1 - t) * Math.max(0, b.h - 2 * PAD);
}
/** y (px) → dB (clamped to the range). */
export function dbAt(b: Box, y: number, range: number): number {
  const t = 1 - (y - PAD) / Math.max(1, b.h - 2 * PAD);
  return clamp(t * 2 * range - range, -range, range);
}
/** Pixels per dB. */
export const pxPerDb = (b: Box, range: number) => Math.max(1, b.h - 2 * PAD) / (2 * range);

/** `n` log-spaced frequencies over the axis (the curve's sample points). */
export function axisFreqs(n: number): number[] {
  return Array.from({ length: n }, (_, i) => tToFreq(i / Math.max(1, n - 1)));
}

/** Frequency caption ("50", "1k", "10k"). */
export function freqLabel(hz: number): string {
  return hz >= 1000 ? `${hz / 1000}k` : `${hz}`;
}

/** New Q after a vertical drag of `dyPx` (up = narrower) or a wheel step. */
export function qAfter(q0: number, dyPx: number): number {
  return q0 * Math.pow(2, -dyPx / 48);
}

/** One band slot with its live bindings. */
export interface ResolvedBand extends GraphicBand {
  index: number;
  used: boolean;
  bindings: { used: ParamBinding; on: ParamBinding; kind: ParamBinding; freq: ParamBinding; gain: ParamBinding; q: ParamBinding; slope: ParamBinding };
}

/** Plain values a drag or a placement has sent but the document may not show yet. */
export type BandDraft = Partial<Pick<ResolvedBand, "used" | "on" | "shape" | "freq" | "gainDb" | "q" | "slope">>;

/** Resolve one slot (`null` when a param is missing from the descriptor). */
export function resolveBand(index: number, spec: GraphicEqBandBinding, bind: (id: number) => ParamBinding | null, draft?: BandDraft): ResolvedBand | null {
  const used = bind(spec.used);
  const on = bind(spec.on);
  const kind = bind(spec.kind);
  const freq = bind(spec.freq);
  const gain = bind(spec.gain);
  const q = bind(spec.q);
  const slope = bind(spec.slope);
  if (!used || !on || !kind || !freq || !gain || !q || !slope) return null;
  const shape: GraphicEqShape = SHAPES[Math.round(kind.plain)] ?? "Bell";
  return {
    index,
    used: used.plain >= 0.5,
    on: on.plain >= 0.5,
    shape,
    freq: freq.plain,
    gainDb: gain.plain,
    q: q.plain,
    slope: Math.round(slope.plain),
    ...draft,
    bindings: { used, on, kind, freq, gain, q, slope },
  };
}

/** Handle position: at the band's frequency and gain (0 dB for shapes without gain). */
export function handleAt(b: Box, band: GraphicBand, range: number): { x: number; y: number } {
  return { x: xOf(b, band.freq), y: yOf(b, usesGain(band.shape) ? band.gainDb : 0, range) };
}

/** The used band whose handle is nearest `(x, y)` within `HIT` px (ties: the selected one). */
export function hitBand(b: Box, bands: ReadonlyArray<ResolvedBand>, x: number, y: number, range: number, selected: number | null): number | null {
  let best: number | null = null;
  let bestScore = Infinity;
  for (const band of bands) {
    if (!band.used) continue;
    const h = handleAt(b, band, range);
    const d = Math.hypot(h.x - x, h.y - y);
    if (d > HIT) continue;
    const score = d - (band.index === selected ? 3 : 0) - (band.on ? 1 : 0);
    if (score < bestScore) {
      bestScore = score;
      best = band.index;
    }
  }
  return best;
}

/** The first unused slot not in `taken`, or `null` when all are in use. */
export function freeSlot(bands: ReadonlyArray<ResolvedBand>, taken: ReadonlySet<number>): number | null {
  for (const b of bands) if (!b.used && !taken.has(b.index)) return b.index;
  return null;
}
