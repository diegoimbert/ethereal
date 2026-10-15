/**
 * Pure math of the effect windows' visualisations (`devices-pass-effects`): the dynamics
 * transfer curve (compressor / gate / expander / limiter), delay echo taps, reverb decay
 * and the MIDI velocity curve. The dynamics and velocity curves follow the devices' own
 * static formulas (`ether_devices::compressor::gain_reduction_knee_db`, the gate's
 * expander/gate law, `fx_velocity::Velocity::map`); the delay and reverb displays are
 * illustrative envelopes of the params.
 */

/** Gain reduction (dB, >= 0) of a compressor at `level` dB: `knee` dB quadratic soft knee. */
export function compressorReductionDb(level: number, threshold: number, ratio: number, knee: number): number {
  const over = level - threshold;
  const slope = 1 - 1 / Math.max(1, ratio);
  if (2 * over <= -knee) return 0;
  if (2 * over >= knee) return over * slope;
  const x = over + knee / 2;
  return (slope * x * x) / (2 * knee);
}

export type DynamicsMode = "compress" | "expand" | "gate" | "limit";

/** Mode from a mode param label ("Peak", "RMS" = compress; "Expand…"; "Gate"). */
export function dynamicsModeOf(label: string | null | undefined, hasRatio: boolean): DynamicsMode {
  const l = (label ?? "").toLowerCase();
  if (l.startsWith("expand")) return "expand";
  if (l === "gate") return "gate";
  return hasRatio ? "compress" : "limit";
}

/** Largest expander reduction (dB), as the compressor's `MAX_EXPAND_DB`. */
const MAX_EXPAND_DB = 60;

export interface DynamicsShape {
  mode: DynamicsMode;
  threshold: number;
  ratio: number;
  knee: number;
  /** Gate / expander floor (dB, <= 0; -80 or lower = silence). */
  floor: number;
  /** Input gain (dB) before the curve (a limiter's Gain). */
  inputGain: number;
}

/** Output level (dB) for an input level (dB) through the static curve. */
export function dynamicsOutDb(s: DynamicsShape, input: number): number {
  const x = input + s.inputGain;
  switch (s.mode) {
    case "compress":
      return x - compressorReductionDb(x, s.threshold, s.ratio, s.knee);
    case "limit":
      return Math.min(x, s.threshold);
    case "expand": {
      const under = s.threshold - x;
      const range = s.floor < 0 ? -s.floor : MAX_EXPAND_DB;
      return under > 0 ? x - Math.min(under * (Math.max(1, s.ratio) - 1), range) : x;
    }
    case "gate":
      return x >= s.threshold ? x : x + s.floor;
  }
}

/** Times (s) and levels (0..1) of the first echoes of a delay (up to `max` or 60 dB down). */
export function delayTaps(timeSeconds: number, feedback: number, freeze: boolean, span: number, max = 32): Array<{ t: number; level: number; side: number }> {
  const out: Array<{ t: number; level: number; side: number }> = [];
  if (!(timeSeconds > 0)) return out;
  const fb = freeze ? 1 : Math.max(0, Math.min(1, feedback));
  let level = 1;
  for (let i = 1; i <= max; i++) {
    const t = i * timeSeconds;
    if (t > span || level < 0.001) break;
    out.push({ t, level, side: (i - 1) % 2 });
    level *= fb;
  }
  return out;
}

/** Note value of a delay division label ("1/8 D", "1/16", "1/4 T", "1/1") in beats. */
export function divisionBeats(label: string | null | undefined): number | null {
  const m = /^\s*(\d+)\s*\/\s*(\d+)\s*([DT])?/i.exec(label ?? "");
  if (!m) return null;
  let beats = (4 * Number(m[1])) / Number(m[2]);
  const mod = m[3]?.toUpperCase();
  if (mod === "D") beats *= 1.5;
  if (mod === "T") beats *= 2 / 3;
  return beats;
}

/** Reverb envelope (0..1 linear amplitude) at `t` seconds: silent during the pre-delay, then
 * an exponential fall of 60 dB over `decay` seconds (`Infinity` = frozen). */
export function reverbEnvelope(t: number, preDelay: number, decay: number): number {
  if (t < preDelay) return 0;
  if (!Number.isFinite(decay)) return 1;
  return Math.pow(10, (-3 * (t - preDelay)) / Math.max(1e-3, decay));
}

export interface VelocityShape {
  drive: number;
  compand: number;
  inLow: number;
  inHigh: number;
  outLow: number;
  outHigh: number;
  mode: "clip" | "gate" | "fixed";
}

/** Output velocity (1..127) for an input velocity, or `null` when `Gate` drops it. */
export function velocityMap(s: VelocityShape, v: number): number | null {
  if (s.mode === "fixed") return s.outHigh;
  const lo = Math.min(s.inLow, s.inHigh);
  const hi = Math.max(s.inLow, s.inHigh);
  if (s.mode === "gate" && (v < lo || v > hi)) return null;
  let x = hi > lo ? (Math.max(lo, Math.min(hi, v)) - lo) / (hi - lo) : 1;
  x = Math.pow(x, Math.pow(4, -s.drive / 100));
  const u = 2 * x - 1;
  const c = Math.sign(u) * Math.pow(Math.abs(u), Math.pow(4, -s.compand / 100));
  x = (c + 1) / 2;
  return s.outLow + x * (s.outHigh - s.outLow);
}

export function velocityModeOf(label: string | null | undefined): VelocityShape["mode"] {
  const l = (label ?? "").toLowerCase();
  return l === "gate" ? "gate" : l === "fixed" ? "fixed" : "clip";
}

/** Saturator curves by label (`ether_devices::fx_color::saturator` `Curve::apply`). */
export function saturatorCurve(label: string | null | undefined): ((u: number) => number) | null {
  const TUBE_K = 1.6;
  switch ((label ?? "").toLowerCase()) {
    case "soft":
      return Math.tanh;
    case "hard":
      return (u) => Math.max(-1, Math.min(1, u));
    case "tube":
      return (u) => (u >= 0 ? 1 - Math.exp(-u) : (Math.exp(TUBE_K * u) - 1) / TUBE_K);
    case "tape":
      return (u) => (2 / Math.PI) * Math.atan((Math.PI / 2) * u);
    case "fold":
      return Math.sin;
    case "rectify":
      return (u) => Math.abs(Math.tanh(u));
    default:
      return null;
  }
}

/**
 * Saturator transfer `x → curve(drive·(x + bias)) − curve(drive·bias)` over -1..1, scaled so
 * the largest |y| on the plotted range is 1 (the display shows the shape, auto gain aside).
 */
export function saturatorTransfer(curve: (u: number) => number, drive: number, bias: number): (x: number) => number {
  const off = curve(drive * bias);
  let peak = 1e-6;
  for (let i = 0; i <= 64; i++) {
    const x = (i / 64) * 2 - 1;
    peak = Math.max(peak, Math.abs(curve(drive * (x + bias)) - off));
  }
  return (x) => Math.max(-1, Math.min(1, (curve(drive * (x + bias)) - off) / peak));
}
