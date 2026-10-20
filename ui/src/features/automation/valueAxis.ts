/**
 * Param-aware value axis of a lane (pure): which values a parameter snaps to, the visible
 * value range and the step gridlines.
 *
 * - **Stepped** params (enums, toggles, semitones) snap to their steps by default and draw
 *   one gridline per step (thinned when they get too dense), labelled where there is room.
 * - **Continuous** params snap to a sensible increment in display units (1 dB, 1 %, 1 st,
 *   0.01 pan, Hz/ms on two significant digits: 1 Hz steps below 100 Hz, 10 Hz to 1 kHz,
 *   100 Hz above...) only while ⇧ is held during a drag or a double-click (see
 *   `snapValue`), so points land on round displayed values; ⇧↑/⇧↓ step to the next one
 *   (`incrementNudge`). ⌥ is precision: off the time grid and 1/10 of the pointer's
 *   motion (`precisionDrag`; ⇧'s increments win for the value); ⌘/Ctrl locks a drag
 *   to one axis.
 * - The **visible range** is a window of normalized values (`ValueRange`): stepped params
 *   open on a window where each step is at least `MIN_STEP_PX` tall, around their default
 *   (so a 64 px Transpose lane shows ±3 st, a taller lane more); the lane's value scale
 *   scrolls and zooms it.
 * - Dragging a stepped param moves it by whole steps at `DRAG_PX_PER_STEP`, whatever the
 *   lane height or window.
 *
 * Steps come from `ParamInfo.step` when the descriptor has the field;
 * enums (`labels`) and the `Toggle` unit are stepped either way. Descriptors from before
 * that field (no `step` key at all) also treat linear `Semitones` as whole semitones; with
 * the field present, `step: null` means continuous and nothing is inferred.
 */

import type { ParamInfo } from "@/generated";
import { formatParam, paramToNormalized, paramToPlain } from "@/domain/paramScale";
import { formatKeys, isMacPlatform } from "@/features/keymap/chords";
import { POINT_DRAG_GESTURES } from "@/features/keymap/groups";
import { t, type MessageKey } from "@/i18n";

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
  // Inference only for descriptors without the field (e.g. old plugins/mocks).
  if (!("step" in info) && info.unit === "Semitones" && info.scale.type === "Linear") return 1;
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
    case "Cents":
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

/**
 * Size of one display increment around `plain` (the step `incrementPlain` rounds to), for
 * stepping to the next round value. Hz/ms/s: the two-significant-digit step there.
 */
export function incrementSize(info: ParamInfo, plain: number): number {
  switch (info.unit) {
    case "Decibels":
    case "Percent":
    case "Semitones":
    case "Cents":
      return 1;
    case "Pan":
      return 0.01;
    case "Ratio":
      return 0.1;
    case "Hertz":
    case "Milliseconds":
    case "Seconds": {
      const a = Math.abs(plain);
      return a > 0 && Number.isFinite(a) ? Math.pow(10, Math.floor(Math.log10(a)) - 1) : 1;
    }
    case "Toggle":
      return Math.abs(info.max - info.min);
    case "None":
      return Math.abs(info.max - info.min) / 100;
  }
}

/**
 * The normalized value one whole increment above (`dir` > 0) or below `normalized`, on a
 * round displayed value (⇧↑ / ⇧↓): 0.4 dB up → 1 dB, 1 dB up → 2 dB; 1 kHz down → 990 Hz.
 * Stepped params move by one step.
 */
export function incrementNudge(info: ParamInfo, normalized: number, dir: number): number {
  const step = paramStep(info);
  const n = stepCount(info);
  if (step !== null && n) return snapValue(info, normalized + Math.sign(dir) / n);
  const lo = Math.min(info.min, info.max);
  const hi = Math.max(info.min, info.max);
  const plain = paramToPlain(info, normalized);
  const here = incrementPlain(info, plain);
  // Already on a round value (or past it in `dir`): one increment further, else to it.
  const sign = Math.sign(dir) * Math.sign(info.max - info.min || 1);
  const onIt = Math.abs(here - plain) <= 1e-9 * Math.max(1, Math.abs(plain)) || Math.sign(here - plain) !== sign;
  // The increment size on the side we move to (990 Hz below 1 kHz, not 900).
  const size = incrementSize(info, plain + sign * 1e-9 * Math.max(1, Math.abs(plain)));
  const next = onIt ? incrementPlain(info, here + sign * size) : here;
  return paramToNormalized(info, Math.min(hi, Math.max(lo, next)));
}

/** What the step modifier snaps a continuous param to, for the drag hint ("1 db"). */
export function incrementLabel(info: ParamInfo): string {
  switch (info.unit) {
    case "Decibels":
      return "1 db";
    case "Percent":
      return "1 %";
    case "Pan":
      return "1 %";
    case "Ratio":
      return "0.1";
    case "Semitones":
      return "1 st";
    case "Cents":
      return "1 ct";
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

/**
 * Modifier hints of a point drag, for the tooltip, from the keymap's point-drag gestures
 * (`⇧ 1 dB steps · ⌥ fine, off grid · ⌘ one axis`; `Ctrl`, `Alt`, `Shift` off macOS).
 * Stepped params have no increments modifier (they always snap).
 */
export function dragHint(info: ParamInfo, mac = isMacPlatform()): string {
  const g = POINT_DRAG_GESTURES;
  const part = (x: { keys: string; hint: MessageKey }, hint = t(x.hint)) => `${formatKeys(x.keys, mac)} ${hint}`;
  const common = `${part(g.precise)} · ${part(g.axis)}`;
  return paramStep(info) !== null ? common : `${part(g.increments, `${incrementLabel(info)} ${t(g.increments.hint)}`)} · ${common}`;
}

/**
 * Snap a normalized value: stepped params always snap to their steps; continuous ones
 * snap to their increment only when `increments` (the step modifier, ⇧) is on.
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
