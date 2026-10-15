/**
 * Touch gesture tuning (#522, the design's §4). One place, so every surface (arranger, ruler,
 * piano roll) behaves alike; `recognizer.test.ts` checks each rule.
 */

/** Movement (CSS px) under which a press is still a tap or a long-press. */
export const SLOP_PX = 8;
/** A press held this long without moving is a long-press (ms). */
export const LONG_PRESS_MS = 450;
/** A second tap within this time (ms) and distance (px) of the first is a double-tap. */
export const DOUBLE_TAP_MS = 300;
export const DOUBLE_TAP_PX = 24;
/** A pan locks to one axis when its first movement is this much more along it than across. */
export const AXIS_LOCK_RATIO = 2;
/** Pinch spreads (px, per axis) under this are clamped: no huge factors from nearly touching fingers. */
export const MIN_PINCH_SPREAD_PX = 24;

/** Momentum: speeds over this window (ms) set the fling speed. */
export const VELOCITY_WINDOW_MS = 80;
/** Momentum starts above this speed (px/ms) and stops under the stop speed. */
export const FLING_MIN_SPEED = 0.15;
export const FLING_STOP_SPEED = 0.02;
/** Exponential decay time constant (ms), like iOS's normal scroll deceleration. */
export const FLING_TIME_CONSTANT_MS = 325;

/** A short haptic tick (Android; Safari has no `navigator.vibrate`). */
export function haptic(ms = 10): void {
  try {
    navigator.vibrate?.(ms);
  } catch {
    // not allowed (no user activation yet) or unsupported
  }
}
