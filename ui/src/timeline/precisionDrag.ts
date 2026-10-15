/**
 * Precision drags (#793): while the precision modifier (⌥ / Option) is held, a point drag moves
 * at a fraction of the pointer's motion, for values a pixel is too coarse for. Shared by the
 * automation lanes (and clip envelopes), the tempo lane and the expression lanes.
 */

/** How far a point moves per pixel of pointer motion while the precision modifier is held. */
export const PRECISION_RATIO = 0.1;

export interface DragOffset {
  dx: number;
  dy: number;
}

/** Which axes move at the precision ratio (a boolean: both). */
export type PrecisionAxes = boolean | { x: boolean; y: boolean };

/**
 * A drag's effective offset from its start, given the raw pointer position `p` and which axes
 * are in precision mode. Each axis accumulates its motion from the last mode change: the anchor
 * is rebased there (at the last pointer position seen), so pressing or releasing the modifier
 * mid-drag never jumps the point. Call it with the drag's start position.
 */
export function precisionDrag(start: { x: number; y: number }, ratio = PRECISION_RATIO): (p: { x: number; y: number }, fine: PrecisionAxes) => DragOffset {
  let last = start;
  const axes = {
    x: { anchor: start.x, base: 0, fine: false },
    y: { anchor: start.y, base: 0, fine: false },
  };
  return (p, fine) => {
    const want = typeof fine === "boolean" ? { x: fine, y: fine } : fine;
    const out = { dx: 0, dy: 0 };
    for (const k of ["x", "y"] as const) {
      const a = axes[k];
      if (want[k] !== a.fine) {
        a.base += (last[k] - a.anchor) * (a.fine ? ratio : 1);
        a.anchor = last[k];
        a.fine = want[k];
      }
      out[k === "x" ? "dx" : "dy"] = a.base + (p[k] - a.anchor) * (a.fine ? ratio : 1);
    }
    last = p;
    return out;
  };
}
