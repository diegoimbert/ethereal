import { describe, expect, it } from "vitest";
import type { LfoCurve, LfoPoint } from "@/generated";
import {
  DEFAULT_LFO_CURVE,
  MAX_LFO_POINTS,
  addPoint,
  curvePath,
  gridDivisions,
  hitPoint,
  lfoValueAt,
  movePoint,
  removePoint,
  sameCurve,
  setSegmentCurve,
  snapX,
} from "./lfoCurveMath";

const lin = (x: number, y: number): LfoPoint => ({ x, y, curve: { type: "Linear" } });
const step = (x: number, y: number): LfoPoint => ({ x, y, curve: { type: "Step" } });

describe("lfo curve math (mirrors ether_model::lfo_curve)", () => {
  it("evaluates segments, the wrap and vertical edges like the engine", () => {
    // The vectors of the Rust unit test `evaluates_segments_wrap_and_edges`.
    const c = [lin(0.25, 0), lin(0.5, 1), step(0.75, 0.5)];
    expect(lfoValueAt(c, 0.375)).toBeCloseTo(0.5, 9);
    expect(lfoValueAt(c, 0.5)).toBeCloseTo(1, 9);
    expect(lfoValueAt(c, 0.9)).toBeCloseTo(0.5, 9);
    expect(lfoValueAt(c, 0.1)).toBeCloseTo(0.5, 9);
    expect(lfoValueAt(c, 1.375)).toBeCloseTo(0.5, 9);
    const edge = [lin(0, 1), lin(0.5, 1), lin(0.5, 0)];
    expect(lfoValueAt(edge, 0.4999)).toBeCloseTo(1, 3);
    expect(lfoValueAt(edge, 0.5)).toBe(0);
    expect(lfoValueAt(edge, 0.75)).toBeCloseTo(0.5, 9);
    // Tension 1 = x⁴ (the automation law).
    expect(lfoValueAt([{ x: 0, y: 0, curve: { type: "Curve", tension: 1 } }, lin(1, 1)], 0.5)).toBeCloseTo(0.5 ** 4, 9);
    expect(lfoValueAt([], 0.3)).toBe(1);
    expect(lfoValueAt([lin(0.2, 0.4)], 0.9)).toBe(0.4);
  });

  it("draws vertical edges exactly", () => {
    const pts = curvePath([step(0, 1), step(0.5, 0)], 100, 10).split(" ");
    expect(pts).toContain("50.0,0.0");
    expect(pts).toContain("50.0,10.0");
    expect(pts[0]).toBe("0.0,0.0");
    expect(pts[pts.length - 1]).toBe("100.0,10.0");
  });

  it("grids the cycle in sixteenths, 4 to 32 cells", () => {
    expect(gridDivisions(1)).toBe(4);
    expect(gridDivisions(4)).toBe(16);
    expect(gridDivisions(32)).toBe(32);
    expect(gridDivisions(1 / 16)).toBe(4);
    expect(gridDivisions(null)).toBe(8);
    expect(snapX(0.37, 4, false)).toBe(0.25);
    expect(snapX(0.37, 4, true)).toBe(0.37);
    expect(snapX(1.2, 4, true)).toBe(1);
  });

  it("adds, moves, removes and reshapes points", () => {
    const c: LfoCurve = DEFAULT_LFO_CURVE;
    const added = addPoint(c, 0.3, 0.7)!;
    expect(added.index).toBe(2);
    expect(added.curve.points.map((p) => p.x)).toEqual([0, 0.02, 0.3, 0.6]);
    // A move stays between the neighbours (the order never changes).
    expect(movePoint(added.curve, 2, 0.9, 2).points[2]).toEqual(lin(0.6, 1));
    expect(movePoint(added.curve, 2, -1, 0.5).points[2]).toEqual(lin(0.02, 0.5));
    expect(removePoint(added.curve, 2)).toEqual(c);
    expect(removePoint({ points: [lin(0, 1)] }, 0).points).toHaveLength(1);
    expect(setSegmentCurve(c, 0, { type: "Step" }).points[0]!.curve).toEqual({ type: "Step" });
    const full: LfoCurve = { points: Array.from({ length: MAX_LFO_POINTS }, (_, i) => lin(i / MAX_LFO_POINTS, 0.5)) };
    expect(addPoint(full, 0.5, 0.5)).toBeNull();
  });

  it("hits the nearest point within the radius, compares curves like f32s", () => {
    const pts = [lin(0, 1), lin(0.5, 0)];
    expect(hitPoint(pts, 98, 9, 200, 10, 6)).toBe(1);
    expect(hitPoint(pts, 50, 5, 200, 10, 6)).toBeNull();
    expect(sameCurve({ points: [lin(0.1, 0.3)] }, { points: [lin(Math.fround(0.1), Math.fround(0.3))] })).toBe(true);
    expect(sameCurve({ points: [lin(0.1, 0.3)] }, { points: [step(0.1, 0.3)] })).toBe(false);
  });
});
