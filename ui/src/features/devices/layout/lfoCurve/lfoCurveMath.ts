/**
 * Volume LFO curve math and edits (pure). Mirrors Rust `ether_model::lfo_curve` exactly:
 * points sorted by `x` (0..1) with `y` (0..1); a point's `CurveShape` shapes the segment
 * to the next point (the automation law, `curveFraction`); the cycle wraps, the last
 * segment running to the first point of the next cycle; points sharing an `x` make a
 * vertical edge (the later one wins from that `x` on). At most 64 points, at least one.
 */

import type { CurveShape, LfoCurve, LfoPoint, ShaperCurves } from "@/generated";
import { shapeFraction } from "@/features/automation/curve";

/** `ether_model::MAX_LFO_POINTS`. */
export const MAX_LFO_POINTS = 64;

const clamp = (v: number, lo: number, hi: number) => (v < lo ? lo : v > hi ? hi : v);
const clamp01 = (v: number) => clamp(v, 0, 1);

/** A new Volume LFO's curve (Rust `LfoCurve::default`): the classic pump. */
export const DEFAULT_LFO_CURVE: LfoCurve = {
  points: [
    { x: 0, y: 1, curve: { type: "Linear" } },
    { x: 0.02, y: 0, curve: { type: "Curve", tension: -0.45 } },
    { x: 0.6, y: 1, curve: { type: "Linear" } },
  ],
};

/** The shaper's default curves (Rust `ShaperCurves::default`). */
export const DEFAULT_SHAPER_CURVES: ShaperCurves = {
  volume: DEFAULT_LFO_CURVE,
  cutoff: {
    points: [
      { x: 0, y: 0.15, curve: { type: "Curve", tension: 0.4 } },
      { x: 0.95, y: 1, curve: { type: "Linear" } },
    ],
  },
  pan: {
    points: [
      { x: 0, y: 0.5, curve: { type: "Linear" } },
      { x: 0.25, y: 1, curve: { type: "Linear" } },
      { x: 0.75, y: 0, curve: { type: "Linear" } },
    ],
  },
};

/** The segment of `pts` covering `p` (0..1): its start point index and its x span (the wrap may start below 0 or end above 1). */
export function segmentAt(pts: ReadonlyArray<LfoPoint>, p: number): { i: number; x0: number; x1: number } {
  const n = pts.length;
  let k = 0; // points with x <= p
  while (k < n && pts[k]!.x <= p) k++;
  if (k === 0) return { i: n - 1, x0: pts[n - 1]!.x - 1, x1: pts[0]!.x };
  if (k === n) return { i: n - 1, x0: pts[n - 1]!.x, x1: pts[0]!.x + 1 };
  return { i: k - 1, x0: pts[k - 1]!.x, x1: pts[k]!.x };
}

/** Value at `phase` (cycles, any real): Rust `lfo_value_at`. No point = 1. */
export function lfoValueAt(pts: ReadonlyArray<LfoPoint>, phase: number): number {
  const n = pts.length;
  if (n === 0) return 1;
  if (n === 1) return pts[0]!.y;
  const p = phase - Math.floor(phase);
  const { i, x0, x1 } = segmentAt(pts, p);
  const a = pts[i]!;
  const b = pts[(i + 1) % n]!;
  const span = x1 - x0;
  if (span <= 0) return b.y;
  return a.y + (b.y - a.y) * shapeFraction(a.curve, clamp01((p - x0) / span));
}

/** SVG points of the cycle in a `w`×`h` box (y up), with exact vertical edges. */
export function curvePath(pts: ReadonlyArray<LfoPoint>, w: number, h: number, gain: (y: number) => number = (y) => y): string {
  const out: string[] = [];
  const put = (x: number, y: number) => out.push(`${(x * w).toFixed(1)},${((1 - gain(y)) * h).toFixed(1)}`);
  if (pts.length === 0) {
    put(0, 1);
    put(1, 1);
    return out.join(" ");
  }
  // Breakpoints of the drawing: 0, each point, 1; curved segments are sampled in between.
  const xs = [0, ...pts.map((p) => p.x), 1];
  for (let k = 0; k < xs.length - 1; k++) {
    const a = xs[k]!;
    const b = xs[k + 1]!;
    // Left limit then value at `a` (they differ on a vertical edge or a step).
    if (k > 0) put(a, lfoValueAt(pts, a - 1e-9));
    put(a, lfoValueAt(pts, a));
    const seg = segmentAt(pts, (a + b) / 2);
    const steps = b > a && pts[seg.i]!.curve.type === "Curve" ? 24 : 1;
    for (let s = 1; s < steps; s++) {
      const x = a + ((b - a) * s) / steps;
      put(x, lfoValueAt(pts, x));
    }
  }
  put(1, lfoValueAt(pts, 1 - 1e-9));
  return out.join(" ");
}

/** Cycle length in beats of each sync rate (Rust `ether_devices::contract::SYNC_RATE_BEATS`, same order). */
export const SYNC_RATE_BEATS: ReadonlyArray<number> = [1 / 16, 1 / 12, 1 / 8, 1 / 6, 1 / 4, 1 / 3, 1 / 2, 2 / 3, 1, 2, 4, 8, 16, 32];

/** Approximately equal curves (the engine stores `f32`s). */
export function sameCurve(a: LfoCurve, b: LfoCurve): boolean {
  const near = (u: number, v: number) => Math.abs(u - v) < 1e-6;
  return (
    a.points.length === b.points.length &&
    a.points.every((p, i) => {
      const q = b.points[i]!;
      return near(p.x, q.x) && near(p.y, q.y) && p.curve.type === q.curve.type && near(tensionOf(p.curve), tensionOf(q.curve));
    })
  );
}

const tensionOf = (c: CurveShape) => (c.type === "Curve" ? c.tension : 0);

/** Grid divisions of one cycle: sixteenth notes (4 to 32 cells); free rate = 8. */
export function gridDivisions(cycleBeats: number | null): number {
  if (cycleBeats === null || !(cycleBeats > 0)) return 8;
  const cells = Math.pow(2, Math.round(Math.log2(cycleBeats * 4)));
  return clamp(cells, 4, 32);
}

/** `x` snapped to `divisions` (unchanged with `bypass`). */
export function snapX(x: number, divisions: number, bypass: boolean): number {
  return clamp01(bypass ? x : Math.round(x * divisions) / divisions);
}

const sorted = (pts: LfoPoint[]): LfoCurve => ({ points: pts });

/** Add a point at (`x`, `y`) with a linear segment after it; `null` when full. */
export function addPoint(curve: LfoCurve, x: number, y: number): { curve: LfoCurve; index: number } | null {
  if (curve.points.length >= MAX_LFO_POINTS) return null;
  const pts = [...curve.points];
  let index = 0;
  while (index < pts.length && pts[index]!.x <= x) index++;
  pts.splice(index, 0, { x: clamp01(x), y: clamp01(y), curve: { type: "Linear" } });
  return { curve: sorted(pts), index };
}

/** Move point `i` to (`x`, `y`), kept between its neighbours (order never changes). */
export function movePoint(curve: LfoCurve, i: number, x: number, y: number): LfoCurve {
  const pts = [...curve.points];
  const p = pts[i];
  if (!p) return curve;
  const lo = i > 0 ? pts[i - 1]!.x : 0;
  const hi = i < pts.length - 1 ? pts[i + 1]!.x : 1;
  pts[i] = { ...p, x: clamp(x, lo, hi), y: clamp01(y) };
  return sorted(pts);
}

/** Remove point `i` (the last point stays). */
export function removePoint(curve: LfoCurve, i: number): LfoCurve {
  if (curve.points.length <= 1 || !curve.points[i]) return curve;
  return sorted(curve.points.filter((_, k) => k !== i));
}

/** Set the shape of the segment starting at point `i`. */
export function setSegmentCurve(curve: LfoCurve, i: number, shape: CurveShape): LfoCurve {
  const p = curve.points[i];
  if (!p) return curve;
  const pts = [...curve.points];
  pts[i] = { ...p, curve: shape };
  return sorted(pts);
}

/** Whether the segment starting at point `i` rises (its end point is higher). */
export function segmentRises(curve: LfoCurve, i: number): boolean {
  const n = curve.points.length;
  return curve.points[(i + 1) % n]!.y >= curve.points[i]!.y;
}

/** Index of the point within `radius` px of (`px`, `py`) in a `w`×`h` box, the nearest, or `null`. */
export function hitPoint(pts: ReadonlyArray<LfoPoint>, px: number, py: number, w: number, h: number, radius: number): number | null {
  let best: number | null = null;
  let dist = radius;
  pts.forEach((p, i) => {
    const d = Math.hypot(p.x * w - px, (1 - p.y) * h - py);
    if (d <= dist) {
      dist = d;
      best = i;
    }
  });
  return best;
}
