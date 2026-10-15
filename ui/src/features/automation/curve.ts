/**
 * Automation curve math. Mirrors Rust `ether_core::automation` exactly (the engine is the
 * source of truth), so what the lane draws is what the engine plays:
 *
 * - `curveFraction(x, tension) = x^(4^tension)`, `tension` clamped to -1..1 and `x` to 0..1.
 *   Tension 0 is linear, +1 is `x⁴` (slow start), -1 is `x^¼` (fast start).
 * - A point's `curve` shapes the segment from that point to the next one. `Step` holds the
 *   value until the next point.
 * - Before the first point the first value holds; after the last point the last one does.
 *
 * `tension` is an `f32` in Rust and is widened to `f64` before use, so we round it through
 * `Math.fround` to get bit-identical inputs. Test vectors: `curveVectors.json` (generated
 * from the Rust functions, see README.md).
 */

import type { CurveShape } from "@/generated";

const clamp = (v: number, lo: number, hi: number) => (v < lo ? lo : v > hi ? hi : v);

/** `ether_core::automation::curve_fraction`. */
export function curveFraction(x: number, tension: number): number {
  const t = clamp(Math.fround(tension), -1, 1);
  return Math.pow(clamp(x, 0, 1), Math.pow(4, t));
}

/** Shaped 0..1 fraction of a segment at `x` (0..1) for the segment's curve. */
export function shapeFraction(curve: CurveShape, x: number): number {
  switch (curve.type) {
    case "Linear":
      return x;
    case "Step":
      return 0;
    case "Curve":
      return curveFraction(x, curve.tension);
  }
}

/** A breakpoint as far as curve evaluation is concerned. */
export interface CurvePoint {
  time: number;
  value: number;
  curve: CurveShape;
}

/**
 * `ether_core::automation::evaluate`: normalized value of a lane at `time`. `points` must
 * be sorted by time. Returns `null` when there are no points.
 */
export function evaluatePoints(points: ReadonlyArray<CurvePoint>, time: number): number | null {
  const first = points[0];
  if (!first) return null;
  if (time <= first.time) return first.value;
  // Index of the first point strictly after `time`.
  let lo = 0;
  let hi = points.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (points[mid]!.time <= time) lo = mid + 1;
    else hi = mid;
  }
  if (lo >= points.length) return points[points.length - 1]!.value;
  const a = points[lo - 1]!;
  const b = points[lo]!;
  const span = b.time - a.time;
  if (span <= 0) return b.value;
  return a.value + (b.value - a.value) * shapeFraction(a.curve, (time - a.time) / span);
}

/** Clamp a tension to the engine's range (and to `f32` precision, as it is stored). */
export function clampTension(tension: number): number {
  return Math.fround(clamp(tension, -1, 1));
}
