import { describe, expect, it } from "vitest";
import type { CurveShape } from "@/generated";
import { clampTension, curveFraction, evaluatePoints, shapeFraction, splitPointAt, type CurvePoint } from "./curve";
import vectors from "./curveVectors.json";

// Rust and JS `pow` may differ in the last ulp; anything visible would be far larger.
const TOL = 1e-12;

describe("curveFraction (mirrors ether_core::automation::curve_fraction)", () => {
  it.each(vectors.curve_fraction as Array<[number, number, number]>)("x=%s tension=%s", (x, tension, expected) => {
    expect(Math.abs(curveFraction(x, tension) - expected)).toBeLessThan(TOL);
  });

  it("is x^(4^tension) with the documented extremes", () => {
    expect(curveFraction(0.5, 0)).toBe(0.5);
    expect(curveFraction(0.5, 1)).toBeCloseTo(0.0625, 15);
    expect(curveFraction(0.5, -1)).toBeCloseTo(Math.pow(0.5, 0.25), 15);
    // Clamped tension and x.
    expect(curveFraction(0.5, 5)).toBe(curveFraction(0.5, 1));
    expect(curveFraction(2, 0.3)).toBe(1);
    expect(curveFraction(-1, -0.3)).toBe(0);
  });

  it("rounds tension through f32 like the engine", () => {
    // 0.3 as f32 widened to f64 is 0.30000001192092896, not 0.3.
    expect(curveFraction(0.5, 0.3)).toBe(Math.pow(0.5, Math.pow(4, Math.fround(0.3))));
    expect(clampTension(0.3)).toBe(Math.fround(0.3));
    expect(clampTension(-4)).toBe(-1);
  });
});

describe("evaluatePoints (mirrors ether_core::automation::evaluate)", () => {
  const points: CurvePoint[] = (vectors.evaluate.points as Array<[number, number, CurveShape]>).map(([time, value, curve]) => ({
    time,
    value,
    curve,
  }));

  it.each(vectors.evaluate.cases as Array<[number, number]>)("t=%s", (time, expected) => {
    expect(Math.abs(evaluatePoints(points, time)! - expected)).toBeLessThan(TOL);
  });

  it("returns null without points and holds a single point", () => {
    expect(evaluatePoints([], 1)).toBeNull();
    expect(evaluatePoints([{ time: 2, value: 0.4, curve: { type: "Linear" } }], 10)).toBe(0.4);
  });

  it("jumps at coincident points", () => {
    const pts: CurvePoint[] = [
      { time: 0, value: 0, curve: { type: "Linear" } },
      { time: 1, value: 0.2, curve: { type: "Linear" } },
      { time: 1, value: 0.8, curve: { type: "Linear" } },
    ];
    expect(evaluatePoints(pts, 1)).toBe(0.8);
  });

  it("shapes by segment curve", () => {
    expect(shapeFraction({ type: "Step" }, 0.9)).toBe(0);
    expect(shapeFraction({ type: "Linear" }, 0.25)).toBe(0.25);
  });
});

describe("splitPointAt: inserting a point keeps the lane", () => {
  const LINEAR: CurveShape = { type: "Linear" };
  const lane = (...pts: Array<[number, number, CurveShape]>): CurvePoint[] => pts.map(([time, value, curve]) => ({ time, value, curve }));
  /** The lane with a point inserted at `t` (after any point at the same time, like a later id). */
  const inserted = (pts: CurvePoint[], t: number, fallback: CurveShape = LINEAR): CurvePoint[] => {
    const p = { time: t, ...splitPointAt(pts, t, fallback)! };
    const i = pts.findIndex((q) => q.time > t);
    return i < 0 ? [...pts, p] : [...pts.slice(0, i), p, ...pts.slice(i)];
  };
  /** Largest difference between two lanes over `[from, to]`. */
  const maxDiff = (a: CurvePoint[], b: CurvePoint[], from: number, to: number) => {
    let worst = 0;
    for (let k = 0; k <= 1000; k++) {
      const t = from + ((to - from) * k) / 1000;
      worst = Math.max(worst, Math.abs(evaluatePoints(a, t)! - evaluatePoints(b, t)!));
    }
    return worst;
  };

  it("linear: the value on the line, the curve unchanged everywhere", () => {
    const pts = lane([0, 0.2, LINEAR], [4, 0.8, LINEAR]);
    expect(splitPointAt(pts, 2, LINEAR)).toEqual({ value: 0.5, curve: LINEAR });
    for (const t of [0.3, 1, 2, 3.7]) expect(maxDiff(pts, inserted(pts, t), -1, 5)).toBeLessThan(1e-12);
  });

  it("step: the held value, still a step", () => {
    const pts = lane([0, 0.2, { type: "Step" }], [4, 0.8, LINEAR]);
    expect(splitPointAt(pts, 3, LINEAR)).toEqual({ value: 0.2, curve: { type: "Step" } });
    expect(maxDiff(pts, inserted(pts, 3), -1, 5)).toBe(0);
  });

  it("toggles (step lanes of 0/1): the held state", () => {
    const STEP: CurveShape = { type: "Step" };
    const pts = lane([0, 1, STEP], [4, 0, STEP], [8, 1, STEP]);
    expect(splitPointAt(pts, 6, STEP)).toEqual({ value: 0, curve: STEP });
    expect(splitPointAt(pts, 12, STEP)).toEqual({ value: 1, curve: STEP });
    expect(maxDiff(pts, inserted(pts, 6, STEP), 0, 12)).toBe(0);
  });

  it("before the first and after the last point: the held value, flat new segment", () => {
    const pts = lane([2, 0.3, LINEAR], [4, 0.9, { type: "Curve", tension: 0.5 }]);
    expect(splitPointAt(pts, 1, { type: "Step" })).toEqual({ value: 0.3, curve: { type: "Step" } });
    expect(splitPointAt(pts, 9, LINEAR)).toEqual({ value: 0.9, curve: LINEAR });
    expect(maxDiff(pts, inserted(pts, 1), 0, 10)).toBe(0);
    expect(maxDiff(pts, inserted(pts, 9), 0, 10)).toBe(0);
  });

  it("on a point's time: that point's value and curve", () => {
    const curve: CurveShape = { type: "Curve", tension: -0.6 };
    const pts = lane([0, 0.1, LINEAR], [2, 0.4, curve], [4, 1, LINEAR]);
    expect(splitPointAt(pts, 2, LINEAR)).toEqual({ value: 0.4, curve });
    expect(maxDiff(pts, inserted(pts, 2), 0, 5)).toBe(0);
  });

  it("curves: the left half is exact, the right half the closest tension", () => {
    for (const tension of [-1, -0.6, -0.3, 0.2, 0.5, 1]) {
      for (const s of [0.1, 0.25, 0.5, 0.75, 0.9]) {
        const pts = lane([0, 0.1, { type: "Curve", tension }], [8, 0.9, LINEAR]);
        const t = 8 * s;
        const after = inserted(pts, t);
        expect(after[1]!.value).toBe(evaluatePoints(pts, t));
        // Left of the new point: the same shape (up to pow rounding).
        expect(maxDiff(pts, after, 0, t)).toBeLessThan(1e-12);
        // Right of it: within ~2.2 % of the segment's 0.8 rise even at the extremes...
        expect(maxDiff(pts, after, t, 8)).toBeLessThan(0.0225 * 0.8);
        // ...and much closer for gentle curves.
        if (Math.abs(tension) <= 0.3) expect(maxDiff(pts, after, t, 8)).toBeLessThan(0.01 * 0.8);
      }
    }
  });

  it("matches the engine (Rust golden vectors) wherever a point is inserted", () => {
    const pts: CurvePoint[] = (vectors.evaluate.points as Array<[number, number, CurveShape]>).map(([time, value, curve]) => ({ time, value, curve }));
    const cases = vectors.evaluate.cases as Array<[number, number]>;
    for (const [time, expected] of cases) {
      expect(Math.abs(splitPointAt(pts, time, LINEAR)!.value - expected)).toBeLessThan(TOL);
      // Inserting on the linear and step segments leaves every golden value as the engine
      // computed it.
      if (time > 8) continue;
      const after = inserted(pts, time);
      for (const [t, e] of cases) expect(Math.abs(evaluatePoints(after, t)! - e)).toBeLessThan(TOL);
    }
  });

  it("no points: null", () => {
    expect(splitPointAt([], 1, LINEAR)).toBeNull();
  });
});
