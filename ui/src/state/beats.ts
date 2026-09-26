/**
 * Beat-time helpers mirroring Rust `ether_model::Beats` exactly
 * (`crates/ether-model/src/value.rs`).
 *
 * Musical time is `f64` beats (quarter notes). Conventions shared by engine and UI:
 * - never compare beats with `===`: use `beatsApproxEq` / `BEATS_EPSILON`;
 * - grid operations (quantize, snapping, bar math) go through `snapBeats` / `floorBeats` /
 *   `ceilBeats`, which absorb float error so a value within `BEATS_EPSILON` of a grid line
 *   is treated as on it.
 * All grid helpers return `x` unchanged when `grid <= 0`.
 */

import type { Beats } from "@/generated";

/** Tolerance for beat comparisons (~1/1000 of a 1/1024 note). */
export const BEATS_EPSILON = 1e-6;

export function beatsApproxEq(a: Beats, b: Beats): boolean {
  return Math.abs(a - b) <= BEATS_EPSILON;
}

/** Nearest multiple of `grid`. */
export function snapBeats(x: Beats, grid: Beats): Beats {
  if (grid <= 0) return x;
  return Math.round(x / grid) * grid;
}

/** Largest multiple of `grid` that is `<= x + BEATS_EPSILON`. */
export function floorBeats(x: Beats, grid: Beats): Beats {
  if (grid <= 0) return x;
  return Math.floor((x + BEATS_EPSILON) / grid) * grid;
}

/** Smallest multiple of `grid` that is `>= x - BEATS_EPSILON`. */
export function ceilBeats(x: Beats, grid: Beats): Beats {
  if (grid <= 0) return x;
  return Math.ceil((x - BEATS_EPSILON) / grid) * grid;
}
