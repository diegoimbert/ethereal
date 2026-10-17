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

/** What a point inserted on a lane at some time must be so the lane doesn't change. */
export interface SplitPoint {
  /** The lane's value there (`evaluatePoints`). */
  value: number;
  /** The new point's curve (it shapes the segment from it to the next point). */
  curve: CurveShape;
}

/** Samples of the right half compared when fitting its tension. */
const FIT_SAMPLES = 32;

/**
 * Tension whose curve best matches the right part (from `s` to 1) of the curve
 * `x^(4^tension)`, rescaled to 0..1: `((s + (1 - s)·u)^k - s^k) / (1 - s^k)`.
 * Golden-section search over -1..1 on the largest error (minimax) at `FIT_SAMPLES` points.
 */
function fitRightTension(tension: number, s: number): number {
  const k = Math.pow(4, clamp(Math.fround(tension), -1, 1));
  const sk = Math.pow(s, k);
  const denom = 1 - sk;
  if (denom <= 0) return tension;
  const target: number[] = [];
  for (let i = 1; i < FIT_SAMPLES; i++) {
    const u = i / FIT_SAMPLES;
    target.push((Math.pow(s + (1 - s) * u, k) - sk) / denom);
  }
  const err = (t: number) => {
    let e = 0;
    for (let i = 1; i < FIT_SAMPLES; i++) {
      const d = curveFraction(i / FIT_SAMPLES, t) - target[i - 1]!;
      e = Math.max(e, Math.abs(d));
    }
    return e;
  };
  const phi = (Math.sqrt(5) - 1) / 2;
  let lo = -1;
  let hi = 1;
  let a = hi - phi * (hi - lo);
  let b = lo + phi * (hi - lo);
  let ea = err(a);
  let eb = err(b);
  for (let i = 0; i < 60; i++) {
    if (ea < eb) {
      hi = b;
      b = a;
      eb = ea;
      a = hi - phi * (hi - lo);
      ea = err(a);
    } else {
      lo = a;
      a = b;
      ea = eb;
      b = lo + phi * (hi - lo);
      eb = err(b);
    }
  }
  return (lo + hi) / 2;
}

/**
 * The point to insert at `time` on a lane (sorted `points`) so the lane keeps its shape:
 * its value is the lane's value there (exactly what the engine plays), and its curve
 * continues the segment it splits. `null` without points.
 *
 * - Before the first point / after the last: the held value; the new segment is flat, so
 *   it takes `fallback` as its curve.
 * - Linear: linear (exact). Step: step, holding the same value (exact).
 * - Curve `x^k`: the left half keeps the segment's tension exactly
 *   (`(s·u)^k = s^k·u^k`). The right half is not a power curve in general, so the new
 *   point gets the tension that fits it best (minimax); the lane then moves by at most
 *   ~2.2 % of the segment's rise (tension ±1), far less for gentle curves (`curve.test.ts`).
 */
export function splitPointAt(points: ReadonlyArray<CurvePoint>, time: number, fallback: CurveShape): SplitPoint | null {
  const value = evaluatePoints(points, time);
  if (value === null) return null;
  // The segment `time` is in: the last point at or before it starts it.
  let i = -1;
  for (let j = 0; j < points.length; j++) {
    if (points[j]!.time <= time) i = j;
    else break;
  }
  const a = points[i];
  const b = points[i + 1];
  if (!a || !b) return { value, curve: fallback };
  const curve = a.curve;
  if (curve.type !== "Curve") return { value, curve };
  const s = (time - a.time) / (b.time - a.time);
  if (!(s > 0) || Math.fround(curve.tension) === 0) return { value, curve };
  return { value, curve: { type: "Curve", tension: clampTension(fitRightTension(curve.tension, s)) } };
}
