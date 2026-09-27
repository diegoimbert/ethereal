/**
 * Param-aware value axis of a lane (pure): which values a parameter snaps to, the visible
 * value range and the step gridlines.
 *
 * - **Stepped** params (enums, toggles, semitones) snap to their steps by default and draw
 *   one gridline per step (thinned when they get too dense), labelled where there is room.
 * - **Continuous** params snap to a sensible increment (1 dB, 1 %, two significant digits
 *   of Hz/ms...) only while the step modifier is held (see `snapValue`).
 * - The **visible range** is a window of normalized values (`ValueRange`): stepped params
 *   open on a window where each step is at least `MIN_STEP_PX` tall, around their default
 *   (so a 64 px Transpose lane shows ±3 st, a taller lane more); the lane's value scale
 *   scrolls and zooms it.
 * - Dragging a stepped param moves it by whole steps at `DRAG_PX_PER_STEP`, whatever the
 *   lane height or window.
 *
 * `ParamInfo` has no step metadata yet (BCR sent): steps are inferred from `labels`, the
 * `Toggle` unit, and `Semitones` with a linear scale (whole semitones).
 */

import type { ParamInfo } from "@/generated";
import { formatParam, paramToNormalized, paramToPlain } from "@/features/devices/paramScale";

/** A window of normalized values, `0 <= lo < hi <= 1`. */
export interface ValueRange {
  lo: number;
  hi: number;
}

export const FULL_RANGE: ValueRange = { lo: 0, hi: 1 };

/** Stepped params open on a window where a step is at least this tall (px). */
export const MIN_STEP_PX = 8;
/** Vertical drag per step of a stepped param (px), independent of the lane height. */
export const DRAG_PX_PER_STEP = 8;
/** Narrowest window (steps for stepped params; else this fraction of the range). */
export const MIN_VISIBLE_STEPS = 4;
export const MIN_VISIBLE_FRACTION = 0.05;

/** Plain step of a stepped param, or `null` when it is continuous. */
export function paramStep(info: ParamInfo): number | null {
  const span = info.max - info.min;
  if (span === 0) return null;
  const withStep = info as ParamInfo & { step?: number | null };
  if (typeof withStep.step === "number" && withStep.step > 0) return withStep.step;
  if (info.labels && info.labels.length > 1) return span / (info.labels.length - 1);
  if (info.unit === "Toggle") return span;
  if (info.unit === "Semitones" && info.scale.type === "Linear") return 1;
  return null;
}

/** Number of steps of a stepped param (`null`: continuous). */
export function stepCount(info: ParamInfo): number | null {
  const step = paramStep(info);
  return step === null ? null : Math.round((info.max - info.min) / step);
}

/** Two significant digits ("nice" rounding for Hz, ms...). */
function roundSignificant(v: number, digits = 2): number {
  if (v === 0 || !Number.isFinite(v)) return v;
  const e = Math.floor(Math.log10(Math.abs(v))) - (digits - 1);
  const f = Math.pow(10, e);
  return Math.round(v / f) * f;
}

/** Nearest "incremental step" plain value of a continuous param (1 dB, 1 %, ...). */
export function incrementPlain(info: ParamInfo, plain: number): number {
  switch (info.unit) {
    case "Decibels":
      return Math.round(plain);
    case "Percent":
      return Math.round(plain);
    case "Pan":
      return Math.round(plain * 100) / 100;
    case "Ratio":
      return Math.round(plain * 10) / 10;
    case "Semitones":
      return Math.round(plain);
    case "Hertz":
    case "Milliseconds":
    case "Seconds":
      return roundSignificant(plain);
    case "Toggle":
      return plain >= (info.min + info.max) / 2 ? info.max : info.min;
    case "None": {
      // 1 % of the range.
      const unit = (info.max - info.min) / 100;
      return info.min + Math.round((plain - info.min) / unit) * unit;
    }
  }
}

/** What the step modifier snaps a continuous param to, for the drag hint ("1 dB"). */
export function incrementLabel(info: ParamInfo): string {
  switch (info.unit) {
    case "Decibels":
      return "1 dB";
    case "Percent":
      return "1 %";
    case "Pan":
      return "1 %";
    case "Ratio":
      return "0.1";
    case "Semitones":
      return "1 st";
    case "Hertz":
    case "Milliseconds":
    case "Seconds":
      return "round values";
    case "Toggle":
      return "on/off";
    case "None":
      return "1 %";
  }
}

/** Modifier hints of a point drag, for the tooltip. */
export function dragHint(info: ParamInfo): string {
  return paramStep(info) !== null ? "⌥ free time · ⇧ one axis" : `⌥ ${incrementLabel(info)} steps, free time · ⇧ one axis`;
}

/**
 * Snap a normalized value: stepped params always snap to their steps; continuous ones
 * snap to their increment only when `increments` (the step modifier) is on.
 */
export function snapValue(info: ParamInfo, normalized: number, increments = false): number {
  const n = Math.min(1, Math.max(0, normalized));
  const step = paramStep(info);
  if (step !== null) {
    const plain = info.min + Math.round((paramToPlain(info, n) - info.min) / step) * step;
    return paramToNormalized(info, plain);
  }
  if (!increments) return n;
  const lo = Math.min(info.min, info.max);
  const hi = Math.max(info.min, info.max);
  const plain = Math.min(hi, Math.max(lo, incrementPlain(info, paramToPlain(info, n))));
  return paramToNormalized(info, plain);
}

/**
 * Normalized size of one keyboard nudge: one step (stepped params) or 1 % of the range;
 * `fine` (shift) is a tenth of that for continuous params.
 */
export function nudgeSize(info: ParamInfo, fine = false): number {
  const n = stepCount(info);
  if (n !== null && n > 0) return 1 / n;
  return fine ? 0.001 : 0.01;
}

/**
 * Opening window of the value axis for a lane `usablePx` tall: stepped params with more
 * steps than fit at `MIN_STEP_PX` show that many steps around `center` (normalized;
 * default: the param's default value); everything else shows its whole range.
 */
export function defaultRange(info: ParamInfo, usablePx: number, center = paramToNormalized(info, info.default)): ValueRange {
  const n = stepCount(info);
  if (n === null || n <= 0) return FULL_RANGE;
  const fit = Math.max(MIN_VISIBLE_STEPS, Math.floor(usablePx / MIN_STEP_PX));
  if (n <= fit) return FULL_RANGE;
  const half = fit / 2 / n;
  return clampRange({ lo: center - half, hi: center + half }, info);
}

/** Normalized change of a vertical drag of `dyUp` px on a stepped param (whole steps). */
export function stepDragDelta(info: ParamInfo, dyUp: number): number | null {
  const n = stepCount(info);
  if (n === null || n <= 0) return null;
  return Math.round(dyUp / DRAG_PX_PER_STEP) / n;
}

/** Keep a window inside 0..1, no narrower than the minimum (sliding it back in). */
export function clampRange(r: ValueRange, info?: ParamInfo): ValueRange {
  const n = info ? stepCount(info) : null;
  const minSpan = n ? Math.min(1, MIN_VISIBLE_STEPS / n) : MIN_VISIBLE_FRACTION;
  let span = Math.min(1, Math.max(minSpan, r.hi - r.lo));
  if (!Number.isFinite(span)) span = 1;
  let lo = r.lo;
  if (lo < 0) lo = 0;
  if (lo + span > 1) lo = 1 - span;
  return { lo, hi: lo + span };
}

/** Scroll a window by `delta` (normalized). */
export function scrollRange(r: ValueRange, delta: number, info?: ParamInfo): ValueRange {
  return clampRange({ lo: r.lo + delta, hi: r.hi + delta }, info);
}

/** Zoom a window by `factor` (> 1 = show more) around the normalized value `at`. */
export function zoomRange(r: ValueRange, factor: number, at: number, info?: ParamInfo): ValueRange {
  const lo = at - (at - r.lo) * factor;
  const hi = at + (r.hi - at) * factor;
  return clampRange({ lo, hi }, info);
}

export function isFullRange(r: ValueRange): boolean {
  return r.lo <= 0 && r.hi >= 1;
}

/** A gridline of the value axis. */
export interface StepLine {
  /** Normalized value. */
  value: number;
  label: string | null;
  /** The param's default (e.g. 0 st): drawn stronger. */
  major: boolean;
}

/** Nice multiples used to thin dense step lines. */
const THIN = [1, 2, 3, 4, 6, 12, 24, 48, 96];

/**
 * Step gridlines of a stepped param inside the visible window, for a lane `usablePx` tall:
 * every step if they are at least `minGapPx` apart, else every 2nd/3rd/4th/6th/12th...;
 * labels where they are `labelGapPx` apart. Continuous params: none.
 */
export function stepLines(info: ParamInfo, range: ValueRange, usablePx: number, minGapPx = 6, labelGapPx = 14): StepLine[] {
  const step = paramStep(info);
  const n = stepCount(info);
  if (step === null || n === null || n <= 0) return [];
  const span = range.hi - range.lo;
  if (span <= 0 || usablePx <= 0) return [];
  // Linear params: px between steps. Non-linear ones: the smallest gap in the window.
  const gapPx = (usablePx / span) * (1 / n);
  const every = THIN.find((k) => gapPx * k >= minGapPx) ?? n;
  const labelEvery = THIN.find((k) => k % every === 0 && gapPx * k >= labelGapPx) ?? null;
  const out: StepLine[] = [];
  const defIndex = Math.round((info.default - info.min) / step);
  for (let i = 0; i <= n; i++) {
    const rel = i - defIndex;
    if (rel % every !== 0) continue;
    const plain = info.min + i * step;
    const value = paramToNormalized(info, plain);
    if (value < range.lo - 1e-9 || value > range.hi + 1e-9) continue;
    const labelled = labelEvery !== null && rel % labelEvery === 0;
    out.push({ value, label: labelled ? formatParam(info, plain) : null, major: i === defIndex });
  }
  return out;
}

/** Normalized value → y px in a lane `height` tall with `pad` px margins, for a window. */
export function rangeValueToY(value: number, height: number, pad: number, range: ValueRange = FULL_RANGE): number {
  const h = Math.max(1, height - 2 * pad);
  const span = Math.max(1e-9, range.hi - range.lo);
  return pad + (1 - (value - range.lo) / span) * h;
}

/** y px → normalized value (clamped to 0..1, not to the window). */
export function rangeYToValue(y: number, height: number, pad: number, range: ValueRange = FULL_RANGE): number {
  const h = Math.max(1, height - 2 * pad);
  const span = range.hi - range.lo;
  return Math.min(1, Math.max(0, range.lo + (1 - (y - pad) / h) * span));
}
