/**
 * Normalized (0..1) ↔ plain param mapping. Mirrors Rust `ether_protocol::devices::
 * scale_to_plain` / `scale_to_normalized` / `ParamInfo::to_plain` exactly (same test
 * vectors in `paramScale.test.ts`). Generic knobs work in normalized space; the document
 * stores plain values.
 */

import type { ParamInfo, ParamScale } from "@/generated";

const clamp = (v: number, lo: number, hi: number) => (v < lo ? lo : v > hi ? hi : v);

export function scaleToPlain(scale: ParamScale, min: number, max: number, normalized: number): number {
  const n = clamp(normalized, 0, 1);
  let v: number;
  switch (scale.type) {
    case "Linear":
      v = min + n * (max - min);
      break;
    case "Log":
      v = min * Math.pow(max / min, n);
      break;
    case "Power":
      v = min + Math.pow(n, scale.exponent) * (max - min);
      break;
    case "Fader": {
      const amp = n * n * n * Math.pow(10, max / 20);
      v = amp <= 0 ? min : Math.max(20 * Math.log10(amp), min);
      break;
    }
  }
  return clamp(v, Math.min(min, max), Math.max(min, max));
}

export function scaleToNormalized(scale: ParamScale, min: number, max: number, plain: number): number {
  if (max === min) return 0;
  const p = clamp(plain, Math.min(min, max), Math.max(min, max));
  let n: number;
  switch (scale.type) {
    case "Linear":
      n = (p - min) / (max - min);
      break;
    case "Log":
      n = Math.log(p / min) / Math.log(max / min);
      break;
    case "Power":
      n = Math.pow((p - min) / (max - min), 1 / scale.exponent);
      break;
    case "Fader":
      n = p <= min ? 0 : Math.cbrt(Math.pow(10, p / 20) / Math.pow(10, max / 20));
      break;
  }
  return clamp(n, 0, 1);
}

/** `ParamInfo::to_plain`: snaps to steps for enum params. */
export function paramToPlain(info: ParamInfo, normalized: number): number {
  const v = scaleToPlain(info.scale, info.min, info.max, normalized);
  const labels = info.labels;
  if (labels && labels.length > 1) {
    const step = (info.max - info.min) / (labels.length - 1);
    return info.min + Math.round((v - info.min) / step) * step;
  }
  return v;
}

/** `ParamInfo::to_normalized`. */
export function paramToNormalized(info: ParamInfo, plain: number): number {
  return scaleToNormalized(info.scale, info.min, info.max, plain);
}

/** Index into `labels` of an enum param's plain value. */
export function labelIndex(info: ParamInfo, plain: number): number {
  const n = info.labels?.length ?? 0;
  if (n < 2) return 0;
  const step = (info.max - info.min) / (n - 1);
  return clamp(Math.round((plain - info.min) / step), 0, n - 1);
}

/** Plain value of the enum param step `index`. */
export function labelValue(info: ParamInfo, index: number): number {
  const n = info.labels?.length ?? 0;
  if (n < 2) return info.min;
  return info.min + (clamp(index, 0, n - 1) * (info.max - info.min)) / (n - 1);
}

function trimNumber(v: number, digits: number): string {
  return Number(v.toFixed(digits)).toString();
}

/** Human-readable plain value ("-6.0 dB", "1.20 kHz", "35 %", "Saw"). */
export function formatParam(info: ParamInfo, plain: number): string {
  if (info.labels && info.labels.length > 0) return info.labels[labelIndex(info, plain)] ?? String(plain);
  switch (info.unit) {
    case "Decibels":
      return formatDb(plain);
    case "Hertz":
      return plain >= 1000 ? `${trimNumber(plain / 1000, 2)} kHz` : `${trimNumber(plain, plain < 100 ? 1 : 0)} Hz`;
    case "Milliseconds":
      return plain >= 1000 ? `${trimNumber(plain / 1000, 2)} s` : `${trimNumber(plain, plain < 10 ? 2 : plain < 100 ? 1 : 0)} ms`;
    case "Seconds":
      return `${trimNumber(plain, 2)} s`;
    case "Percent":
      return `${trimNumber(plain, 0)} %`;
    case "Semitones":
      return `${plain > 0 ? "+" : ""}${trimNumber(plain, 1)} st`;
    case "Ratio":
      return `${trimNumber(plain, 1)} : 1`;
    case "Pan":
      return formatPan(plain);
    case "Toggle":
      return plain >= 0.5 ? "On" : "Off";
    case "None":
      return trimNumber(plain, 2);
  }
}

/** Decibels with -inf at/below silence (-144 dB, `Decibels::SILENCE`). */
export function formatDb(db: number): string {
  if (db <= SILENCE_DB) return "-inf dB";
  return `${db.toFixed(1)} dB`;
}

/** Pan -1..1 as "C", "25L", "100R". */
export function formatPan(pan: number): string {
  const p = Math.round(pan * 100);
  if (p === 0) return "C";
  return p < 0 ? `${-p}L` : `${p}R`;
}

/** `Decibels::SILENCE` (treated as -inf). */
export const SILENCE_DB = -144;
