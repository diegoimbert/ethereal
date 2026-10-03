/**
 * Geometry of the IR widget: the impulse response as the reverb plays it, i.e. after
 * Pre-delay, Size (time stretch), Decay (tail cut with a raised-cosine fade) and Reverse.
 * Mirrors `ether_devices::fx_space::ir::IrBase::shaped` so the picture is the sound.
 */

import type { FactoryIr, IrSource, PeakData } from "@/generated";

/** Param ids of the Convolution Reverb (`fx_space::convolution_reverb`). */
export const IR_PARAMS = {
  mix: 0,
  preDelay: 1,
  decay: 2,
  size: 3,
  lowCut: 4,
  highCut: 5,
  width: 6,
  gain: 7,
  reverse: 8,
} as const;

/** `Size` range (time stretch) and the longest pre-delay, in the units of the params. */
export const SIZE_MAX = 1.5;
export const PRE_DELAY_MAX_MS = 250;

export interface Shaping {
  /** Fraction of the IR kept, 0.1..=1. */
  decay: number;
  /** Time stretch, 0.5..=1.5. */
  size: number;
  reverse: boolean;
  /** Seconds. */
  preDelay: number;
}

/** Shaping from the plain param values. */
export function shapingOf(values: { decay: number; size: number; reverse: number; preDelay: number }): Shaping {
  return {
    decay: Math.min(1, Math.max(0.1, values.decay / 100)),
    size: Math.min(SIZE_MAX, Math.max(0.5, values.size / 100)),
    reverse: values.reverse >= 0.5,
    preDelay: Math.max(0, values.preDelay) / 1000,
  };
}

/** Fraction of the kept IR covered by the Decay fade (as the engine: 0 at 100 %). */
export function fadeFraction(decay: number): number {
  return Math.min(0.5, (1 - decay) * 5);
}

/** Seconds of IR heard (stretched and cut), and of the stretched IR before the cut. */
export function shapedLength(length: number, s: Shaping): { kept: number; stretched: number } {
  return { kept: length * s.size * s.decay, stretched: length * s.size };
}

/** The time axis of the plot (seconds): fixed per IR, so Size and Pre-delay visibly move. */
export function axisSeconds(length: number): number {
  return length * SIZE_MAX + PRE_DELAY_MAX_MS / 1000;
}

/** Envelope (0..1) of the original IR at `t` seconds. */
export type Envelope = (t: number) => number;

/**
 * A factory IR's envelope (they are synthesized: ~60 dB of exponential decay over their
 * length after a short build-up), for drawing only.
 */
export function factoryEnvelope(ir: FactoryIr): Envelope {
  const len = Math.max(1e-3, ir.length);
  return (t) => (t < 0 || t > len ? 0 : Math.exp((-6.9 * t) / len) * (1 - Math.exp(-t / 0.01)));
}

/** A media IR's envelope from overview peaks (max |sample| over the channels), normalized. */
export function peaksEnvelope(peaks: PeakData, sampleRate: number): Envelope {
  const n = peaks.max[0]?.length ?? 0;
  const amp = new Float32Array(n);
  let top = 0;
  for (let i = 0; i < n; i++) {
    let a = 0;
    for (let c = 0; c < peaks.max.length; c++) a = Math.max(a, Math.abs(peaks.max[c]![i]!), Math.abs(peaks.min[c]![i]!));
    amp[i] = a;
    top = Math.max(top, a);
  }
  const spp = peaks.samples_per_peak;
  const scale = top > 0 ? 1 / top : 0;
  return (t) => {
    const i = Math.floor((t * sampleRate) / spp);
    return i < 0 || i >= n ? 0 : amp[i]! * scale;
  };
}

/** Envelope of the shaped IR at `tau` seconds after the pre-delay. */
export function shapedAt(env: Envelope, length: number, s: Shaping, tau: number): number {
  const { kept } = shapedLength(length, s);
  if (tau < 0 || tau >= kept) return 0;
  const m = s.reverse ? kept - tau : tau;
  const f = fadeFraction(s.decay);
  const fadeStart = kept * (1 - f);
  const fade = f > 0 && m >= fadeStart ? 0.5 * (1 + Math.cos((Math.PI * Math.min(1, (m - fadeStart) / Math.max(1e-9, kept - fadeStart))))) : 1;
  return env(m / s.size) * fade;
}

/** SVG path of the shaped envelope (mirrored around the middle) over `w`×`h` px. */
export function envelopePath(env: Envelope, length: number, s: Shaping, w: number, h: number): string {
  const axis = axisSeconds(length);
  const cols = Math.max(2, Math.round(w));
  const mid = h / 2;
  const top: string[] = [];
  const bottom: string[] = [];
  for (let x = 0; x <= cols; x++) {
    const t = (x / cols) * axis - s.preDelay;
    const a = Math.min(1, shapedAt(env, length, s, t)) * mid * 0.95;
    const px = ((x / cols) * w).toFixed(1);
    top.push(`${px},${(mid - a).toFixed(1)}`);
    bottom.push(`${px},${(mid + a).toFixed(1)}`);
  }
  return `M${top.join("L")}L${bottom.reverse().join("L")}Z`;
}

/** x (px) of a time (seconds) on the plot. */
export function timeToX(t: number, length: number, w: number): number {
  return (t / axisSeconds(length)) * w;
}

/** Time (seconds) at x (px). */
export function xToTime(x: number, length: number, w: number): number {
  return (x / Math.max(1, w)) * axisSeconds(length);
}

/** `Decay` (percent) for a tail end dragged to `x`. */
export function decayAt(x: number, length: number, s: Shaping, w: number): number {
  const stretched = length * s.size;
  const kept = xToTime(x, length, w) - s.preDelay;
  return Math.min(100, Math.max(10, (kept / Math.max(1e-9, stretched)) * 100));
}

/** `Pre-delay` (ms) for its marker dragged to `x`. */
export function preDelayAt(x: number, length: number, w: number): number {
  return Math.min(PRE_DELAY_MAX_MS, Math.max(0, xToTime(x, length, w) * 1000));
}

/** Option value of the IR picker for `ir`. */
export function irKey(ir: IrSource | null): string {
  if (!ir) return "none";
  return ir.type === "Factory" ? `factory:${ir.id}` : `media:${ir.media}`;
}

/** The IR an option value stands for (`undefined`: not an IR option). */
export function irFromKey(key: string): IrSource | null | undefined {
  if (key === "none") return null;
  if (key.startsWith("factory:")) return { type: "Factory", id: key.slice("factory:".length) };
  if (key.startsWith("media:")) return { type: "Media", media: key.slice("media:".length) };
  return undefined;
}

/** The factory IR `step` places after `current` (wrapping; from none: the first/last). */
export function stepFactory(irs: ReadonlyArray<FactoryIr>, current: IrSource | null, step: 1 | -1): IrSource | null {
  if (irs.length === 0) return current;
  const i = current?.type === "Factory" ? irs.findIndex((f) => f.id === current.id) : -1;
  const next = i < 0 ? (step > 0 ? 0 : irs.length - 1) : (i + step + irs.length) % irs.length;
  return { type: "Factory", id: irs[next]!.id };
}

/** Readable seconds ("0.35 s", "2.8 s"). */
export function formatSeconds(s: number): string {
  return `${s < 1 ? s.toFixed(2) : s.toFixed(1)} s`;
}
