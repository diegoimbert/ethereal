/**
 * Magnitude response of the graphic EQ's bands: the TS mirror of
 * `ether_protocol::eq_response::{graphic_eq_stages, graphic_eq_db}`. A band is up to eight
 * second-order stages in series, each the matched design shared with the parametric EQ
 * (`../eq/eqResponse.ts`), so the drawn curve is what the device plays.
 *
 * Parity: `response.test.ts` checks every vector of
 * `crates/ether-protocol/tests/fixtures/graphic_eq_vectors.json` within 1e-6 dB.
 */

import type { GraphicEqShape } from "@/generated";
import { matched, prototype, svfMagnitude, type Prototype, type SvfCoefs, type Zeros } from "../eq/eqResponse";

/** Band types in `Type` label order (Rust `GraphicEqShape::ALL`: plain value = index). */
export const SHAPES: ReadonlyArray<GraphicEqShape> = ["Bell", "LowShelf", "LowCut", "HighShelf", "HighCut", "Notch", "BandPass", "TiltShelf", "FlatTilt"];

/** Slopes (dB/oct) in `Slope` label order (Rust `GRAPHIC_EQ_SLOPES`). */
export const SLOPES: ReadonlyArray<number> = [6, 12, 18, 24, 36, 48, 72, 96];

const MAX_STAGES = 8;
const FLAT_TILT_RANGE: readonly [number, number] = [20, 20000];
const FLAT_TILT_STAGES = 7;
const FLAT_TILT_Q = 0.55;
const SQRT_HALF = Math.SQRT1_2;

/** The gain changes the response. */
export const usesGain = (s: GraphicEqShape) => s === "Bell" || s === "LowShelf" || s === "HighShelf" || s === "TiltShelf" || s === "FlatTilt";
/** The Q changes the response. */
export const usesQ = (s: GraphicEqShape) => s !== "FlatTilt";
/** The slope changes the response. */
export const usesSlope = (s: GraphicEqShape) => s === "LowCut" || s === "HighCut" || s === "BandPass";

function stage(p: Prototype, fs: number, zeros: Zeros): SvfCoefs {
  return matched({ ...p, fp: Math.min(Math.max(p.fp, 1), fs * 0.49) }, fs, zeros, false);
}

const scaled = (c: SvfCoefs, s: number): SvfCoefs => ({ ...c, m0: c.m0 * s, m1: c.m1 * s, m2: c.m2 * s });

/** Linear magnitude of `stages` in series at `atHz`. */
export function stagesMagnitude(stages: ReadonlyArray<SvfCoefs>, atHz: number, sampleRate: number): number {
  let m = 1;
  for (const c of stages) m *= svfMagnitude(c, atHz, sampleRate);
  return m;
}

function normalizeAt(out: SvfCoefs[], atHz: number, fs: number): void {
  const m = stagesMagnitude(out, atHz, fs);
  if (out.length > 0 && m > 0 && Number.isFinite(m)) out[0] = scaled(out[0]!, 1 / m);
}

function push(out: SvfCoefs[], c: SvfCoefs): void {
  if (out.length < MAX_STAGES) out.push(c);
}

function cut(out: SvfCoefs[], low: boolean, fc: number, q: number, order: number, fs: number): void {
  const n = Math.min(Math.max(order, 1), 2 * MAX_STAGES);
  if (n % 2 === 1) {
    const p: Prototype = low ? { fp: fc, k: 2, m0: 1, m1: -1, m2: -1 } : { fp: fc, k: 2, m0: 0, m1: 1, m2: 1 };
    push(out, stage(p, fs, { type: "Matched" }));
  }
  const pairs = Math.floor(n / 2);
  for (let i = 0; i < pairs; i++) {
    const bq = 1 / (2 * Math.cos((Math.PI * (2 * i + 1 + (n % 2))) / (2 * n)));
    const sq = i + 1 === pairs ? (bq * q) / SQRT_HALF : bq;
    push(out, stage(prototype(low ? "LowCut" : "HighCut", fc, 0, sq), fs, low ? { type: "HighPass" } : { type: "Matched" }));
  }
}

/** The stages of one band (Rust `graphic_eq_stages`; `slope` = index in `SLOPES`). */
export function graphicEqStages(shape: GraphicEqShape, freq: number, gainDb: number, q: number, slope: number, sampleRate: number): SvfCoefs[] {
  const fs = sampleRate;
  const fc = Math.min(Math.max(freq, 1), fs * 0.49);
  const qq = Math.max(q, 0.01);
  const slopeDb = SLOPES[Math.min(Math.max(Math.round(slope), 0), SLOPES.length - 1)]!;
  const out: SvfCoefs[] = [];
  switch (shape) {
    case "Bell":
    case "LowShelf":
    case "HighShelf":
      if (gainDb !== 0) push(out, stage(prototype(shape, fc, gainDb, qq), fs, { type: "Matched" }));
      break;
    case "Notch":
      push(out, stage(prototype("Notch", fc, 0, qq), fs, { type: "Notch", fc }));
      break;
    case "LowCut":
    case "HighCut":
      cut(out, shape === "LowCut", fc, qq, Math.floor(slopeDb / 6), fs);
      break;
    case "BandPass":
      if (slopeDb <= 12) push(out, stage(prototype("BandPass", fc, 0, qq), fs, { type: "Matched" }));
      else {
        const spread = Math.pow(2, 0.5 / qq);
        const side = Math.floor(slopeDb / 12);
        cut(out, true, fc / spread, SQRT_HALF, side, fs);
        cut(out, false, fc * spread, SQRT_HALF, side, fs);
        normalizeAt(out, fc, fs);
      }
      break;
    case "TiltShelf":
      if (gainDb !== 0) push(out, scaled(stage(prototype("HighShelf", fc, 2 * gainDb, qq), fs, { type: "Matched" }), Math.pow(10, -gainDb / 20)));
      break;
    case "FlatTilt":
      if (gainDb !== 0) {
        const [lo, hi] = FLAT_TILT_RANGE;
        const step = (2 * gainDb) / FLAT_TILT_STAGES;
        for (let i = 0; i < FLAT_TILT_STAGES; i++) {
          const f = lo * Math.pow(hi / lo, (i + 0.5) / FLAT_TILT_STAGES);
          push(out, stage(prototype("HighShelf", f, step, FLAT_TILT_Q), fs, { type: "Matched" }));
        }
        normalizeAt(out, fc, fs);
      }
      break;
  }
  return out;
}

/** dB of linear magnitude `m` (floored at -120 dB, like Rust). */
const toDb = (m: number) => Math.max(20 * Math.log10(Math.max(m, 1e-6)), -120);

/** Magnitude in dB of one band at `atHz` (Rust `graphic_eq_db`). */
export function graphicEqDb(shape: GraphicEqShape, freq: number, gainDb: number, q: number, slope: number, atHz: number, sampleRate: number): number {
  return toDb(stagesMagnitude(graphicEqStages(shape, freq, gainDb, q, slope, sampleRate), atHz, sampleRate));
}

/** One band's settings as the widget resolves them. */
export interface GraphicBand {
  shape: GraphicEqShape;
  freq: number;
  gainDb: number;
  q: number;
  slope: number;
  on: boolean;
}

/** dB of `band` at each of `freqs` (all zeros when off). */
export function bandCurve(band: GraphicBand, freqs: ReadonlyArray<number>, sampleRate: number): number[] {
  if (!band.on) return freqs.map(() => 0);
  const st = graphicEqStages(band.shape, band.freq, band.gainDb, band.q, band.slope, sampleRate);
  return freqs.map((f) => toDb(stagesMagnitude(st, f, sampleRate)));
}
