/**
 * Expression curve geometry: value ↔ y, SVG paths that follow the engine's interpolation
 * (`evaluatePoints` from the automation feature, which mirrors `ether_core::automation`),
 * and pencil-stroke sampling.
 */

import type { ExpressionPoint } from "@/generated";
import { evaluatePoints } from "@/features/automation";
import type { Range } from "./model";

/** Vertical padding inside a lane, px. */
export const LANE_PAD = 4;

export function valueToY(v: number, range: Range, height: number): number {
  const f = (v - range[0]) / (range[1] - range[0]);
  return LANE_PAD + (1 - f) * (height - 2 * LANE_PAD);
}

export function yToValue(y: number, range: Range, height: number): number {
  const f = 1 - (y - LANE_PAD) / Math.max(1, height - 2 * LANE_PAD);
  return Math.min(range[1], Math.max(range[0], range[0] + f * (range[1] - range[0])));
}

/** Index of the first point with `time >= t`. */
function lowerBound(points: ReadonlyArray<ExpressionPoint>, t: number): number {
  let lo = 0;
  let hi = points.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (points[mid]!.time < t) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

/** Sub-segments drawn per `Curve` segment. */
const CURVE_STEPS = 16;

/**
 * SVG path `d` of a curve between view x `x0` and `x1` (holding the first/last value outside
 * the points). `toX` maps curve time to x, `toTime` the inverse; only the points that touch
 * the visible span are walked.
 */
export function curvePath(
  points: ReadonlyArray<ExpressionPoint>,
  toX: (t: number) => number,
  toTime: (x: number) => number,
  toY: (v: number) => number,
  x0: number,
  x1: number,
): string {
  if (points.length === 0) return "";
  const t0 = toTime(x0);
  const t1 = toTime(x1);
  const first = Math.max(0, lowerBound(points, t0) - 1);
  const last = Math.min(points.length - 1, lowerBound(points, t1));
  const parts: string[] = [];
  const at = (x: number, y: number) => `${x.toFixed(1)},${y.toFixed(1)}`;
  const p0 = points[first]!;
  parts.push(`M${at(Math.min(x0, toX(p0.time)), toY(p0.value))}`, `L${at(toX(p0.time), toY(p0.value))}`);
  for (let i = first; i < last; i++) {
    const a = points[i]!;
    const b = points[i + 1]!;
    const xa = toX(a.time);
    const xb = toX(b.time);
    switch (a.curve.type) {
      case "Step":
        parts.push(`L${at(xb, toY(a.value))}`, `L${at(xb, toY(b.value))}`);
        break;
      case "Linear":
        parts.push(`L${at(xb, toY(b.value))}`);
        break;
      case "Curve":
        for (let k = 1; k <= CURVE_STEPS; k++) {
          const t = a.time + ((b.time - a.time) * k) / CURVE_STEPS;
          parts.push(`L${at(xa + ((xb - xa) * k) / CURVE_STEPS, toY(evaluatePoints([a, b], t) ?? b.value))}`);
        }
        break;
    }
  }
  const pl = points[last]!;
  parts.push(`L${at(Math.max(x1, toX(pl.time)), toY(pl.value))}`);
  return parts.join(" ");
}

/**
 * Pencil stroke samples, bucketed by `minGap` beats so going back over a spot replaces it
 * (the latest value wins) and a stroke never holds more than one point per bucket.
 */
export class StrokeSampler {
  private readonly buckets = new Map<number, { time: number; value: number }>();

  constructor(readonly minGap: number) {}

  add(time: number, value: number): void {
    const t = Math.max(0, time);
    this.buckets.set(Math.round(t / this.minGap), { time: t, value });
  }

  samples(): { time: number; value: number }[] {
    return [...this.buckets.values()];
  }
}
