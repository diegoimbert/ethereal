/**
 * Value helpers of the shared renderer: step snapping (`ParamInfo::snap`), normalized ↔
 * plain with snapping, and the display text honouring `step`, `unit` and `labels`.
 */

import type { ParamInfo } from "@/generated";
import { formatParam, paramToNormalized, paramToPlain } from "../paramScale";

const clamp = (v: number, lo: number, hi: number) => (v < lo ? lo : v > hi ? hi : v);

/** Mirror of Rust `ParamInfo::snap`: clamp, then `min + round((v - min) / step) · step`. */
export function snapPlain(info: ParamInfo, plain: number): number {
  const lo = Math.min(info.min, info.max);
  const hi = Math.max(info.min, info.max);
  const v = clamp(plain, lo, hi);
  const s = info.step;
  if (s === undefined || !(s > 0)) return v;
  // Round away float noise so e.g. 0.1 steps give 0.3, not 0.30000000000000004.
  const snapped = info.min + Math.round((v - info.min) / s) * s;
  return clamp(Number(snapped.toFixed(stepDecimals(s) + 2)), lo, hi);
}

/** Normalized 0..1 → plain, snapped to labels (enums) and to `step`. */
export function toPlain(info: ParamInfo, normalized: number): number {
  return snapPlain(info, paramToPlain(info, normalized));
}

/** Plain → normalized 0..1. */
export function toNormalized(info: ParamInfo, plain: number): number {
  return paramToNormalized(info, plain);
}

/** Decimal places of a step (1 → 0, 0.5 → 1, 0.01 → 2). */
export function stepDecimals(step: number): number {
  if (!(step > 0) || Number.isInteger(step)) return 0;
  const s = step.toString();
  const e = s.indexOf("e-");
  if (e >= 0) return Number(s.slice(e + 2));
  const dot = s.indexOf(".");
  return dot < 0 ? 0 : s.length - dot - 1;
}

/**
 * Display text of a plain value: labels for enums, the unit's format otherwise, snapped to
 * `step` first so stepped params never show fractions ("+7 st", "3", not "2.97").
 */
export function formatValue(info: ParamInfo, plain: number): string {
  if (info.labels && info.labels.length > 0) return formatParam(info, plain);
  const step = info.step;
  if (step === undefined || !(step > 0)) return formatParam(info, plain);
  const v = snapPlain(info, plain);
  const d = stepDecimals(step);
  switch (info.unit) {
    case "None":
      return v.toFixed(d);
    case "Semitones":
      return `${v > 0 ? "+" : ""}${v.toFixed(d)} st`;
    case "Percent":
      return `${v.toFixed(d)} %`;
    default:
      return formatParam(info, v);
  }
}

/** Bipolar params (range across zero) draw their arc from the centre. */
export function isBipolar(info: ParamInfo): boolean {
  return info.min < 0 && info.max > 0;
}

/**
 * Keyboard/arrow increment in normalized units: one step for stepped params (so ↑ moves
 * exactly one semitone/voice), else 1 %.
 */
export function normalizedStep(info: ParamInfo): number {
  const s = info.step;
  if (s && s > 0 && info.max !== info.min) return Math.min(1, s / Math.abs(info.max - info.min));
  return 0.01;
}
