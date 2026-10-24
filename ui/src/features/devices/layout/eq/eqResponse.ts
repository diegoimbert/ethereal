/**
 * Exact magnitude response of the EQ's bands: the TS mirror of
 * `ether_protocol::eq_response::{svf_coefs, svf_magnitude, magnitude_db}` (CONTRACTS.md
 * §12.15). Every shape is a TPT state-variable filter `y = m0·x + m1·band + m2·low`; its
 * digital magnitude at `f` is `|m0 + (m1·s + m2)/(s² + k·s + 1)|` at `s = j·tan(π·f/fs)/g`.
 * `*24` shapes are two identical stages (magnitude squared). The coefficients are the
 * decramped (matched) design: poles by impulse invariance, numerator matching the analog
 * magnitude at DC, Nyquist and the centre (see the Rust module docs).
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

/** Analog prototype: `H(s) = m0 + (m1·s + m2)/(s² + k·s + 1)`, `s = j·f/fp`. */
interface Prototype {
  fp: number;
  k: number;
  m0: number;
  m1: number;
  m2: number;
}

function prototype(shape: EqShape, fc: number, gainDb: number, q: number): Prototype {
  const qq = Math.max(q, 0.01);
  const k = 1 / qq;
  const a = Math.pow(10, gainDb / 40);
  switch (shape) {
    case "LowCut":
    case "LowCut24":
      return { fp: fc, k, m0: 1, m1: -k, m2: -1 };
    case "HighCut":
    case "HighCut24":
      return { fp: fc, k, m0: 0, m1: 0, m2: 1 };
    case "Notch":
      return { fp: fc, k, m0: 1, m1: -k, m2: 0 };
    case "BandPass":
      return { fp: fc, k, m0: 0, m1: k, m2: 0 };
    case "Bell": {
      const kb = 1 / (qq * a);
      return { fp: fc, k: kb, m0: 1, m1: kb * (a * a - 1), m2: 0 };
    }
    case "LowShelf":
      return { fp: fc / Math.sqrt(a), k, m0: 1, m1: k * (a - 1), m2: a * a - 1 };
    case "HighShelf":
      return { fp: fc * Math.sqrt(a), k, m0: a * a, m1: k * (1 - a) * a, m2: 1 - a * a };
  }
}

/** Complex analog response `[re, im]` at `f` Hz. */
function protoAt(p: Prototype, f: number): [number, number] {
  const w = f / p.fp;
  const dr = 1 - w * w;
  const di = p.k * w;
  const nr = p.m2;
  const ni = p.m1 * w;
  const den = dr * dr + di * di;
  return [p.m0 + (nr * dr + ni * di) / den, (ni * dr - nr * di) / den];
}

function protoMag2(p: Prototype, f: number): number {
  const [re, im] = protoAt(p, f);
  return re * re + im * im;
}

/** Coefficients of one stage (cutoff clamped to `1..=0.49·fs`, `q >= 0.01`). */
export function svfCoefs(shape: EqShape, freq: number, gainDb: number, q: number, sampleRate: number): SvfCoefs {
  const fs = sampleRate;
  const fc = Math.min(Math.max(freq, 1), fs * 0.49);
  const p = prototype(shape, fc, gainDb, q);
  const w0 = (2 * Math.PI * p.fp) / fs;
  const zeta = p.k * 0.5;
  const decay = Math.exp(-zeta * w0);
  const a1 = zeta <= 1 ? -2 * decay * Math.cos(Math.sqrt(1 - zeta * zeta) * w0) : -2 * decay * Math.cosh(Math.sqrt(zeta * zeta - 1) * w0);
  const a2 = decay * decay;
  const lo = 1 + a1 + a2;
  const hi = 1 - a1 + a2;
  const g = Math.sqrt(Math.max(lo / hi, 1e-30));
  const k = (2 * (1 - a2)) / (hi * g);
  if (gainDb === 0 && (shape === "Bell" || shape === "LowShelf" || shape === "HighShelf")) return { g, k, m0: 1, m1: 0, m2: 0 };
  const A = [lo * lo, hi * hi, -4 * a2] as const;
  let b0: number, b1: number, b2: number;
  if (shape === "Notch") {
    const c = -2 * Math.cos((2 * Math.PI * fc) / fs);
    const scale = lo / (2 + c);
    [b0, b1, b2] = [scale, scale * c, scale];
  } else {
    const wm = Math.min(w0, Math.PI * 0.5);
    const big0 = protoMag2(p, 0) * A[0];
    const big1 = protoMag2(p, fs * 0.5) * A[1];
    const s = Math.pow(Math.sin(wm * 0.5), 2);
    const phi0 = 1 - s;
    const phi1 = s;
    const polyAt = A[0] * phi0 + A[1] * phi1 + A[2] * 4 * phi0 * phi1;
    const big2 = (protoMag2(p, (wm * fs) / (2 * Math.PI)) * polyAt - big0 * phi0 - big1 * phi1) / (4 * phi0 * phi1);
    const sign = (x: number) => (x < 0 ? -1 : 1);
    const w = Math.sqrt(Math.max(big0, 0)) * sign(protoAt(p, 0)[0]);
    const v = Math.sqrt(Math.max(big1, 0)) * sign(protoAt(p, fs * 0.5)[0]);
    b1 = (w - v) * 0.5;
    const sum = (w + v) * 0.5;
    const r = Math.sqrt(Math.max(sum * sum + big2, 0));
    b0 = sum >= 0 ? (sum + r) * 0.5 : (sum - r) * 0.5;
    b2 = sum - b0;
  }
  const n = 1 + g * k + g * g;
  const m0 = ((b0 - b1 + b2) * n) / 4;
  const m2 = ((b0 + b1 + b2) * n - m0 * 4 * g * g) / (4 * g * g);
  const m1 = (b0 * n - m0 * n - m2 * g * g) / g;
  return { g, k, m0, m1, m2 };
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
