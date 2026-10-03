import { describe, expect, it } from "vitest";
import type { ExpressionPoint } from "@/generated";
import { curvePath, StrokeSampler, valueToY, yToValue } from "./curve";
import {
  checkPoints,
  expressionRange,
  formatValue,
  kindLabel,
  movePoint,
  noteExpressionRange,
  replaceRange,
  strokePoints,
  withPoint,
} from "./model";

const pt = (time: number, value: number): ExpressionPoint => ({ time, value, curve: { type: "Linear" } });

describe("expression model", () => {
  it("ranges match the Rust value table", () => {
    expect(expressionRange({ type: "PitchBend" })).toEqual([-1, 1]);
    expect(expressionRange({ type: "Cc", controller: 1 })).toEqual([0, 1]);
    expect(expressionRange({ type: "ChannelPressure" })).toEqual([0, 1]);
    expect(noteExpressionRange("Pitch")).toEqual([-96, 96]);
    expect(noteExpressionRange("Pressure")).toEqual([0, 1]);
  });

  it("checkPoints: sorted, finite, in range, tension, size", () => {
    expect(checkPoints([pt(0, 0), pt(0, 1), pt(2, 0.5)], [0, 1])).toBeNull();
    expect(checkPoints([pt(1, 0), pt(0, 0)], [0, 1])).toMatch(/sorted/);
    expect(checkPoints([pt(0, 2)], [0, 1])).toMatch(/outside/);
    expect(checkPoints([pt(-1, 0)], [0, 1])).toMatch(/finite/);
    expect(checkPoints([{ time: 0, value: 0, curve: { type: "Curve", tension: 1.5 } }], [0, 1])).toMatch(/tension/);
    expect(checkPoints(Array.from({ length: 16_385 }, () => pt(0, 0)), [0, 1])).toMatch(/at most/);
  });

  it("replaceRange splices [start, end) and rejects points outside it", () => {
    const old = [pt(0, 0), pt(1, 0.2), pt(2, 0.4), pt(3, 0.6)];
    expect(replaceRange(old, 1, 3, [pt(1.5, 1)])).toEqual([pt(0, 0), pt(1.5, 1), pt(3, 0.6)]);
    expect(replaceRange(old, 1, 1, [])).toEqual(old);
    expect(replaceRange(old, 1, 2, [pt(2, 0)])).toMatch(/inside/);
    expect(replaceRange(old, 2, 1, [])).toMatch(/invalid/);
  });

  it("strokePoints: sorted, clamped, thinned, the range covers every point", () => {
    const s = strokePoints(
      [
        { time: 1, value: 0.5 },
        { time: 0, value: -3 },
        { time: 0.01, value: 0.2 },
        { time: 2, value: 3 },
      ],
      [-1, 1],
      0.1,
    )!;
    expect(s.points.map((p) => [p.time, p.value])).toEqual([
      [0, -1],
      [1, 0.5],
      [2, 1],
    ]);
    expect(s.start).toBe(0);
    expect(s.end).toBeGreaterThan(2);
    expect(replaceRange([], s.start, s.end, s.points)).toEqual(s.points);
    expect(strokePoints([], [0, 1], 0.1)).toBeNull();
  });

  it("withPoint / movePoint keep the curve sorted", () => {
    const pts = [pt(0, 0), pt(2, 1)];
    expect(withPoint(pts, pt(1, 0.5)).map((p) => p.time)).toEqual([0, 1, 2]);
    expect(withPoint(pts, pt(2, 0.3))).toEqual([pt(0, 0), pt(2, 0.3)]);
    // Clamped between neighbours and to the range.
    expect(movePoint(withPoint(pts, pt(1, 0.5)), 1, 5, 2, [0, 1])[1]).toEqual(pt(2, 1));
  });

  it("labels and MIDI-facing values", () => {
    expect(kindLabel({ type: "Cc", controller: 1 })).toBe("CC 1 Mod Wheel");
    expect(kindLabel({ type: "Cc", controller: 20 })).toBe("CC 20");
    expect(kindLabel({ type: "PitchBend" })).toBe("Pitch Bend");
    expect(formatValue({ type: "PitchBend" }, -1)).toBe("-8191");
    expect(formatValue({ type: "Cc", controller: 7 }, 0.5)).toBe("64");
    expect(formatValue("Pitch", 2)).toBe("+2.00 st");
  });
});

describe("expression curve geometry", () => {
  const range = [-1, 1] as const;
  it("value ↔ y round-trips inside the padded lane", () => {
    expect(valueToY(1, range, 72)).toBe(4);
    expect(valueToY(-1, range, 72)).toBe(68);
    expect(yToValue(valueToY(0.25, range, 72), range, 72)).toBeCloseTo(0.25);
    expect(yToValue(-50, range, 72)).toBe(1);
  });

  it("curvePath holds outside the points and follows step/linear segments", () => {
    const pts: ExpressionPoint[] = [{ time: 1, value: 0, curve: { type: "Step" } }, pt(2, 1)];
    const d = curvePath(pts, (t) => t * 10, (x) => x / 10, (v) => 10 - v * 10, 0, 40);
    expect(d).toBe("M0.0,10.0 L10.0,10.0 L20.0,10.0 L20.0,0.0 L40.0,0.0");
    expect(curvePath([], (t) => t, (x) => x, (v) => v, 0, 10)).toBe("");
  });

  it("StrokeSampler: the latest sample per bucket wins", () => {
    const s = new StrokeSampler(0.1);
    s.add(1, 0.2);
    s.add(1.01, 0.8);
    s.add(-1, 0.5);
    expect(s.samples()).toEqual([
      { time: 1.01, value: 0.8 },
      { time: 0, value: 0.5 },
    ]);
  });
});
