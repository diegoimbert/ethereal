import type { FadeCurve } from "@/generated";

/**
 * Gain (0..=1) at normalized position `x` in a fade-in (0 = silent start, 1 = full level).
 * A fade-out at normalized position `y` uses `fadeGain(curve, 1 - y)`. Mirrors Rust
 * `ether_core::fades::fade_gain` exactly (keep in sync).
 */
export function fadeGain(curve: FadeCurve, x: number): number {
  const t = Math.min(1, Math.max(0, x));
  switch (curve.type) {
    case "Linear":
      return t;
    case "EqualPower":
      return Math.sin((t * Math.PI) / 2);
    case "Curve": {
      // Same law as automation `CurveShape::Curve`: x^(4^tension).
      const tension = Math.min(1, Math.max(-1, curve.tension));
      return Math.pow(t, Math.pow(4, tension));
    }
  }
}
