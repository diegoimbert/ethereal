/**
 * Exact magnitude response of the EQ's bands: the TS mirror of
 * `ether_protocol::eq_response::{svf_coefs, svf_magnitude, magnitude_db}` (CONTRACTS.md
 * §12.15). Every shape is a TPT state-variable filter `y = m0·x + m1·band + m2·low`; its
 * digital magnitude at `f` is the analog prototype at `s = j·tan(π·f/fs)/g`. `*24` shapes are
 * two identical stages (magnitude squared).
 *
 * Parity: `eqResponse.test.ts` checks every vector of
 * `crates/ether-protocol/tests/fixtures/eq_response_vectors.json` within 1e-6 dB.
 */

import type { EqShape } from "@/generated";

export interface SvfCoefs {
  g: number;
  k: number;
  m0: number;
  m1: number;
  m2: number;
}

/** Coefficients of one stage (cutoff clamped to `1..=0.49·fs`, `q >= 0.01`). */
export function svfCoefs(shape: EqShape, freq: number, gainDb: number, q: number, sampleRate: number): SvfCoefs {
  const fc = Math.min(Math.max(freq, 1), sampleRate * 0.49);
  const g = Math.tan((Math.PI * fc) / sampleRate);
  const qq = Math.max(q, 0.01);
  const k = 1 / qq;
  const a = Math.pow(10, gainDb / 40);
  switch (shape) {
    case "LowCut":
    case "LowCut24":
      return { g, k, m0: 1, m1: -k, m2: -1 };
    case "HighCut":
    case "HighCut24":
      return { g, k, m0: 0, m1: 0, m2: 1 };
    case "Notch":
      return { g, k, m0: 1, m1: -k, m2: 0 };
    case "BandPass":
      return { g, k, m0: 0, m1: k, m2: 0 };
    case "Bell": {
      const kb = 1 / (qq * a);
      return { g, k: kb, m0: 1, m1: kb * (a * a - 1), m2: 0 };
    }
    case "LowShelf":
      return { g: g / Math.sqrt(a), k, m0: 1, m1: k * (a - 1), m2: a * a - 1 };
    case "HighShelf":
      return { g: g * Math.sqrt(a), k, m0: a * a, m1: k * (1 - a) * a, m2: 1 - a * a };
  }
}

/** Linear magnitude of one stage at `atHz`. */
export function svfMagnitude(c: SvfCoefs, atHz: number, sampleRate: number): number {
  const f = Math.min(Math.max(atHz, 0), sampleRate * 0.4999);
  const w = Math.tan((Math.PI * f) / sampleRate) / c.g;
  const dr = 1 - w * w;
  const di = c.k * w;
  const nr = c.m0 * (1 - w * w) + c.m2;
  const ni = w * (c.m0 * c.k + c.m1);
  return Math.sqrt((nr * nr + ni * ni) / (dr * dr + di * di));
}

const steep = (shape: EqShape) => shape === "LowCut24" || shape === "HighCut24";

/** Magnitude in dB of precomputed coefficients (floored at -120 dB). */
export function coefsDb(shape: EqShape, c: SvfCoefs, atHz: number, sampleRate: number): number {
  let m = svfMagnitude(c, atHz, sampleRate);
  if (steep(shape)) m *= m;
  return Math.max(20 * Math.log10(Math.max(m, 1e-6)), -120);
}

/** Magnitude in dB of `shape` at `atHz` (floored at -120 dB). */
export function magnitudeDb(shape: EqShape, freq: number, gainDb: number, q: number, atHz: number, sampleRate: number): number {
  return coefsDb(shape, svfCoefs(shape, freq, gainDb, q, sampleRate), atHz, sampleRate);
}

/** One band's settings, as the widget resolves them from its params. */
export interface EqBand {
  shape: EqShape;
  freq: number;
  gainDb: number;
  q: number;
  on: boolean;
}

/** Per-band dB at each of `freqs` (bands in order; off bands are all zeros). */
export function bandCurves(bands: ReadonlyArray<EqBand>, freqs: ReadonlyArray<number>, sampleRate: number): number[][] {
  return bands.map((b) => {
    if (!b.on) return freqs.map(() => 0);
    const c = svfCoefs(b.shape, b.freq, b.gainDb, b.q, sampleRate);
    return freqs.map((f) => coefsDb(b.shape, c, f, sampleRate));
  });
}

/** The combined response: the sum of each enabled band's dB at each of `freqs`. */
export function summedDb(curves: ReadonlyArray<ReadonlyArray<number>>, n: number): number[] {
  const out = new Array<number>(n).fill(0);
  for (const c of curves) for (let i = 0; i < n; i++) out[i]! += c[i]!;
  return out;
}
