/**
 * Pure math of the typed widgets' previews (filter response, waveshaper transfer, oscillator
 * and LFO shapes, log-frequency axis). Previews are illustrative: they follow the params
 * (mode labels, cutoff, resonance, drive...) but are not bit-exact models of each device's
 * DSP (the EQ's exact response lives in `graphical-eq`'s `eqResponse.ts`).
 */

export const F_MIN = 20;
export const F_MAX = 20000;

/** Frequency → 0..1 on a log axis (20 Hz..20 kHz). */
export function freqToX(f: number): number {
  const v = Math.log(Math.max(F_MIN, Math.min(F_MAX, f)) / F_MIN) / Math.log(F_MAX / F_MIN);
  return v;
}

/** 0..1 on the log axis → frequency. */
export function xToFreq(x: number): number {
  return F_MIN * Math.pow(F_MAX / F_MIN, Math.max(0, Math.min(1, x)));
}

export type FilterKind = "lowpass" | "highpass" | "bandpass" | "notch" | "peak" | "lowshelf" | "highshelf";

export interface FilterShape {
  kind: FilterKind;
  /** 24 dB/oct variants: the 12 dB response squared. */
  steep: boolean;
}

/** Filter shape from a mode label ("Low-pass 24", "HP", "Band-pass", "Notch", "Peak"...). */
export function filterShapeOf(label: string | null | undefined): FilterShape {
  const l = (label ?? "").toLowerCase();
  const steep = /24|48|4-pole|4 pole/.test(l);
  const kind: FilterKind = /shelf/.test(l)
    ? /high|hi/.test(l)
      ? "highshelf"
      : "lowshelf"
    : /notch/.test(l)
      ? "notch"
      : /band|bp/.test(l)
        ? "bandpass"
        : /peak|bell/.test(l)
          ? "peak"
          : /high|hp/.test(l)
            ? "highpass"
            : "lowpass";
  return { kind, steep };
}

/** Q of a resonance param given as 0..1 (normalized): 0.5 (no peak) .. 12. */
export function resonanceToQ(n: number): number {
  const r = Math.max(0, Math.min(1, n));
  return 0.5 * Math.pow(24, r);
}

/** Magnitude in dB of a 2nd-order analog prototype at `f` (s = j·f/fc). */
export function filterMagnitudeDb(shape: FilterShape, f: number, fc: number, q: number, gainDb = 0): number {
  const w = f / Math.max(1e-6, fc);
  // Denominator s² + s/Q + 1 with s = jw → (1 - w²) + j(w/Q).
  const dRe = 1 - w * w;
  const dIm = w / q;
  const den = Math.hypot(dRe, dIm);
  let mag: number;
  switch (shape.kind) {
    case "lowpass":
      mag = 1 / den;
      break;
    case "highpass":
      mag = (w * w) / den;
      break;
    case "bandpass":
      mag = w / q / den;
      break;
    case "notch":
      mag = Math.abs(1 - w * w) / den;
      break;
    case "peak": {
      const a = Math.pow(10, gainDb / 40);
      mag = Math.hypot(1 - w * w, (w * a) / q) / Math.hypot(1 - w * w, w / (a * q));
      break;
    }
    case "lowshelf":
    case "highshelf": {
      // First-order-ish shelf: blend between 0 dB and gain around fc.
      const t = shape.kind === "lowshelf" ? 1 / (1 + w * w) : (w * w) / (1 + w * w);
      return gainDb * t;
    }
  }
  const db = 20 * Math.log10(Math.max(mag, 1e-9));
  return shape.steep ? 2 * db : db;
}

/** Waveshaper transfer: `drive` gain ≥ 1, `curve` 0 (soft, tanh) .. 1 (hard clip), `bias` -1..1. */
export function transfer(x: number, drive: number, curve = 0, bias = 0): number {
  const k = Math.max(1, drive);
  const soft = (v: number) => Math.tanh(v);
  const hard = (v: number) => Math.max(-1, Math.min(1, v));
  const shape = (v: number) => (1 - curve) * soft(v) + curve * hard(v);
  // Remove the DC offset the bias introduces so the curve passes through the origin.
  const y = shape(k * (x + bias)) - shape(k * bias);
  const norm = Math.max(1e-6, Math.max(Math.abs(shape(k * (1 + bias)) - shape(k * bias)), Math.abs(shape(k * (-1 + bias)) - shape(k * bias))));
  return Math.max(-1, Math.min(1, y / norm));
}

export type WaveKind = "sine" | "triangle" | "saw" | "ramp-down" | "square" | "pulse" | "noise" | "random" | "table";

/** Wave shape from a shape label ("Sine", "Saw Down", "S&H", "Wavetable"...). */
export function waveOf(label: string | null | undefined): WaveKind {
  const l = (label ?? "").toLowerCase();
  if (/table|wt/.test(l)) return "table";
  if (/s&h|sample|random|rand/.test(l)) return "random";
  if (/noise/.test(l)) return "noise";
  if (/tri/.test(l)) return "triangle";
  if (/down|ramp/.test(l)) return "ramp-down";
  if (/saw/.test(l)) return "saw";
  if (/pulse/.test(l)) return "pulse";
  if (/square|sqr/.test(l)) return "square";
  return "sine";
}

/** Deterministic pseudo-random -1..1 for step `i` (stable previews). */
function hash(i: number): number {
  const s = Math.sin(i * 12.9898 + 78.233) * 43758.5453;
  return (s - Math.floor(s)) * 2 - 1;
}

/**
 * One sample of a wave at `phase` (cycles, any real), -1..1. `morph` 0..1 is the wavetable
 * position (sine → triangle → saw → square across the table) or the pulse width.
 */
export function waveAt(kind: WaveKind, phase: number, morph = 0.5): number {
  const p = phase - Math.floor(phase);
  switch (kind) {
    case "sine":
      return Math.sin(2 * Math.PI * p);
    case "triangle":
      return p < 0.25 ? 4 * p : p < 0.75 ? 2 - 4 * p : 4 * p - 4;
    case "saw":
      return 2 * p - 1;
    case "ramp-down":
      return 1 - 2 * p;
    case "square":
      return p < 0.5 ? 1 : -1;
    case "pulse":
      return p < 0.05 + 0.9 * Math.max(0, Math.min(1, morph)) ? 1 : -1;
    case "noise":
      return hash(Math.floor(phase * 64));
    case "random":
      return hash(Math.floor(phase * 4));
    case "table": {
      const order: WaveKind[] = ["sine", "triangle", "saw", "square"];
      const pos = Math.max(0, Math.min(1, morph)) * (order.length - 1);
      const i = Math.min(order.length - 2, Math.floor(pos));
      const t = pos - i;
      return (1 - t) * waveAt(order[i]!, phase) + t * waveAt(order[i + 1]!, phase);
    }
  }
}

/** SVG polyline points of `f` sampled at `n + 1` points across `w`. */
export function samplePath(n: number, w: number, f: (t: number) => number): string {
  const out: string[] = [];
  for (let i = 0; i <= n; i++) {
    const t = i / n;
    out.push(`${(t * w).toFixed(2)},${f(t).toFixed(2)}`);
  }
  return out.join(" ");
}
