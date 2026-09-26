/**
 * Grid: resolution (fixed or adaptive to zoom), grid lines and snapping.
 *
 * A resolved grid step is either a musical length inside the bar (`{ beats }`, e.g. 0.25 =
 * a 1/16 note, counted from each bar line) or a number of whole bars (`{ bars }`, counted
 * from bar 1). Bar-relative stepping keeps the grid correct across time-signature changes
 * and odd meters (7/8). All rounding goes through the shared helpers in `@/state/beats`.
 */

import type { Beats, BeatRange, TimeSignature } from "@/generated";
import { BEATS_EPSILON, ceilBeats, floorBeats, snapBeats } from "@/state/beats";
import { beatsPerBar, beatUnit, type TempoMap } from "./tempoMap";

/** A resolved grid step. */
export type GridStep = { kind: "beats"; beats: Beats } | { kind: "bars"; bars: number };

/** How dense an adaptive grid is (minimum line spacing, Ableton's "Narrowest...Widest"). */
export type GridDensity = "narrowest" | "narrow" | "medium" | "wide" | "widest";

/** Minimum pixels between adaptive grid lines per density. */
export const DENSITY_MIN_PX: Record<GridDensity, number> = {
  narrowest: 6,
  narrow: 12,
  medium: 24,
  wide: 48,
  widest: 96,
};

/** User grid setting of a view. */
export type GridSetting =
  | { type: "Adaptive"; density: GridDensity; triplet: boolean }
  | { type: "Fixed"; step: GridStep; triplet: boolean }
  | { type: "Off" };

export const DEFAULT_GRID: GridSetting = { type: "Adaptive", density: "medium", triplet: false };

/** Sub-bar steps tried by the adaptive grid, finest first (1/64 note .. half note). */
const SUB_BAR_STEPS: Beats[] = [1 / 16, 1 / 8, 1 / 4, 1 / 2, 1, 2];

/** Largest bar multiple tried by the adaptive grid. */
const MAX_BARS = 1024;

/** Length in beats of a grid step under a signature. */
export function stepLength(step: GridStep, sig: TimeSignature): Beats {
  return step.kind === "beats" ? step.beats : step.bars * beatsPerBar(sig);
}

/**
 * The finest step whose lines are at least `minPx` apart at `pxPerBeat`. Sub-bar steps
 * must be shorter than the bar (and are multiplied by 2/3 when `triplet`); past that, the
 * grid switches to 1, 2, 4, ... bars.
 */
export function adaptiveStep(pxPerBeat: number, minPx: number, sig: TimeSignature, triplet = false): GridStep {
  const bar = beatsPerBar(sig);
  for (const s of SUB_BAR_STEPS) {
    const beats = triplet ? (s * 2) / 3 : s;
    if (beats >= bar - BEATS_EPSILON) break;
    if (beats * pxPerBeat >= minPx) return { kind: "beats", beats };
  }
  let bars = 1;
  while (bars < MAX_BARS && bars * bar * pxPerBeat < minPx) bars *= 2;
  return { kind: "bars", bars };
}

/**
 * Resolve a grid setting at a zoom level. `sig` is the signature used to decide between
 * sub-bar and bar steps (typically the one at the visible start). `null` = snapping off.
 */
export function resolveGrid(setting: GridSetting, pxPerBeat: number, sig: TimeSignature): GridStep | null {
  switch (setting.type) {
    case "Off":
      return null;
    case "Adaptive":
      return adaptiveStep(pxPerBeat, DENSITY_MIN_PX[setting.density], sig, setting.triplet);
    case "Fixed":
      return setting.triplet && setting.step.kind === "beats"
        ? { kind: "beats", beats: (setting.step.beats * 2) / 3 }
        : setting.step;
  }
}

/** Level of a grid line, for styling. */
export type GridLineLevel = "bar" | "beat" | "sub";

export interface GridLine {
  beats: Beats;
  level: GridLineLevel;
  /** 1-based bar number (bar lines only). */
  bar?: number;
}

/**
 * Grid lines in `[range.start, range.end)`. With a sub-bar step, every bar line plus the
 * subdivisions (`"beat"` on the signature's beats, `"sub"` otherwise). With a bar step,
 * bar lines every `bars` bars. Capped at `maxLines` as a safety net.
 */
export function gridLines(tempo: TempoMap, range: BeatRange, step: GridStep, maxLines = 5000): GridLine[] {
  const out: GridLine[] = [];
  if (step.kind === "bars") {
    for (const b of tempo.barLines(range.start, range.end, step.bars)) {
      if (out.length >= maxLines) break;
      out.push({ beats: b.beats, level: "bar", bar: b.bar });
    }
    return out;
  }
  if (step.beats <= 0) return out;
  let bar = tempo.barAt(range.start);
  while (bar.beats < range.end - BEATS_EPSILON && out.length < maxLines) {
    const len = beatsPerBar(bar.signature);
    const unit = beatUnit(bar.signature);
    if (bar.beats >= range.start - BEATS_EPSILON) out.push({ beats: bar.beats, level: "bar", bar: bar.bar });
    const first = Math.max(step.beats, ceilBeats(range.start - bar.beats, step.beats));
    for (let rel = first; rel < len - BEATS_EPSILON && out.length < maxLines; rel += step.beats) {
      const beats = bar.beats + rel;
      if (beats >= range.end - BEATS_EPSILON) break;
      const onBeat = Math.abs(rel - snapBeats(rel, unit)) <= BEATS_EPSILON;
      out.push({ beats, level: onBeat ? "beat" : "sub" });
    }
    bar = tempo.nextBar(bar);
  }
  return out;
}

export type SnapMode = "nearest" | "floor" | "ceil";

/**
 * Snap a position to the grid. `step === null` (grid off) returns `beats` unchanged.
 * Sub-bar steps snap relative to the containing bar line (the bar end is a valid target);
 * bar steps snap to bar lines `1, 1 + bars, ...`.
 */
export function snapToGrid(beats: Beats, step: GridStep | null, tempo: TempoMap, mode: SnapMode = "nearest"): Beats {
  if (step === null) return beats;
  if (step.kind === "beats") {
    if (step.beats <= 0) return beats;
    const bar = tempo.barAt(beats);
    const len = beatsPerBar(bar.signature);
    const rel = beats - bar.beats;
    const snapped =
      mode === "floor" ? floorBeats(rel, step.beats) : mode === "ceil" ? ceilBeats(rel, step.beats) : snapBeats(rel, step.beats);
    // The bar end is also a line (odd meters: 7/8 on a 1-beat grid ends at 3.5).
    const nearerEnd = mode === "nearest" && Math.abs(len - rel) < Math.abs(snapped - rel);
    return bar.beats + (nearerEnd ? len : Math.min(snapped, len));
  }
  const n = Math.max(1, Math.round(step.bars));
  let bar = tempo.barAt(beats);
  // Walk back to an aligned bar (bar - 1 divisible by n).
  let prev = bar;
  let guard = 0;
  while ((((prev.bar - 1) % n) + n) % n !== 0 && guard++ < n) prev = tempo.barAt(prev.beats - BEATS_EPSILON * 2);
  const onLine = Math.abs(beats - prev.beats) <= BEATS_EPSILON;
  if (mode === "floor" || onLine) return prev.beats;
  bar = prev;
  for (let i = 0; i < n; i++) bar = tempo.nextBar(bar);
  if (mode === "ceil") return bar.beats;
  return beats - prev.beats < bar.beats - beats ? prev.beats : bar.beats;
}

/**
 * Snap a drag: the new position of an item that started at `origin` and was dragged by
 * `delta` beats. Absolute (default): the item's new start lands on the grid. Relative:
 * the item keeps its offset from the grid (the delta is snapped instead), like Ableton
 * when moving an off-grid clip. Returns the snapped delta.
 */
export function snapDelta(
  origin: Beats,
  delta: Beats,
  step: GridStep | null,
  tempo: TempoMap,
  relative = false,
): Beats {
  if (step === null) return delta;
  if (!relative) return snapToGrid(origin + delta, step, tempo) - origin;
  const len = stepLength(step, tempo.signatureAt(origin));
  return snapBeats(delta, len);
}

/** Human-readable name of a grid step ("1/16", "1/8T", "1 Bar", "4 Bars"). */
export function formatGridStep(step: GridStep | null): string {
  if (step === null) return "Off";
  if (step.kind === "bars") return step.bars === 1 ? "1 Bar" : `${step.bars} Bars`;
  // Note value = 4 / beats (quarter = 1 beat → 1/4). Triplets: 4 / (beats * 3/2).
  const straight = 4 / step.beats;
  const isPow2 = (x: number) => Math.abs(x - Math.round(x)) < 1e-6 && Math.round(x) > 0 && (Math.round(x) & (Math.round(x) - 1)) === 0;
  if (isPow2(straight)) {
    const d = Math.round(straight);
    return d === 1 ? "1/1" : `1/${d}`;
  }
  const triplet = 4 / ((step.beats * 3) / 2);
  if (isPow2(triplet)) return `1/${Math.round(triplet)}T`;
  return `${+step.beats.toFixed(4)} beats`;
}
