import { describe, expect, it } from "vitest";
import type { CurveShape } from "@/generated";
import { clampTension, curveFraction, evaluatePoints, shapeFraction, type CurvePoint } from "./curve";
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
