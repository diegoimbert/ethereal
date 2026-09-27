/**
 * Detects the second press of a double-click at `pointerdown` time, so a double-click can
 * start a drag without releasing (double-click-and-drag to insert a clip or a note).
 * `dblclick` only fires after the second release, too late for that.
 */

const DOUBLE_PRESS_MS = 400;
const DOUBLE_PRESS_PX = 5;

export type DoublePressDetector = (e: { clientX: number; clientY: number; detail?: number }, key?: string) => boolean;

/**
 * Returns a detector: call it on every primary-button press; it returns true for the
 * second press of a double-click (same `key`, close in time and space) and records the
 * press otherwise.
 */
export function createDoublePress(): DoublePressDetector {
  let last: { key: string; time: number; x: number; y: number } | null = null;
  return (e, key = "") => {
    const now = performance.now();
    const double =
      last !== null &&
      last.key === key &&
      now - last.time <= DOUBLE_PRESS_MS &&
      Math.hypot(e.clientX - last.x, e.clientY - last.y) <= DOUBLE_PRESS_PX;
    last = double ? null : { key, time: now, x: e.clientX, y: e.clientY };
    return double;
  };
}
