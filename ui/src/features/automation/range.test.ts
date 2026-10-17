import { describe, expect, it } from "vitest";
import type { AutomationPoint, CurveShape, PointEdit, PointSpec } from "@/generated";
import { rangeWriteCommand, sectionMovePlan, sliceRange, valueAt, valueBefore, writeRange, type RangePoint, type SectionPlan } from "./range";
import { evaluatePoints } from "./curve";
import { moveEdits } from "./edit";
import { keyedStep } from "@/features/arrangement/editMath";

const lane = (pts: Array<[number, number]>): AutomationPoint[] =>
  pts.map(([time, value], i) => ({ id: `p${i}`, lane: "L", time, value, curve: { type: "Linear" } }));

/** The lane after a write (sorted like `pointsOfLane`: time, then id). */
function apply(pts: AutomationPoint[], w: { remove: string[]; add: PointSpec[] }): RangePoint[] {
  const kept = pts.filter((p) => !w.remove.includes(p.id));
  return [...kept, ...w.add.map((p) => ({ ...p, lane: "L" }))].sort((a, b) => a.time - b.time || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
}

let n = 0;
const ids = () => `z${String(n++).padStart(6, "0")}`;

describe("automation range edits", () => {
  it("left and right limits", () => {
    const pts: RangePoint[] = [
      { time: 0, value: 0, curve: { type: "Linear" } },
      { time: 4, value: 1, curve: { type: "Step" } },
      { time: 8, value: 0.5, curve: { type: "Linear" } },
    ];
    expect(valueAt(pts, 2)).toBeCloseTo(0.5, 12);
    expect(valueBefore(pts, 4)).toBeCloseTo(1, 12);
    expect(valueAt(pts, 6)).toBe(1);
    expect(valueBefore(pts, 8)).toBe(1);
    expect(valueAt(pts, 8)).toBe(0.5);
    expect(valueAt([], 1)).toBeNull();
  });

  it("delete keeps both sides and ramps across the range", () => {
    const pts = lane([
      [0, 0],
      [4, 1],
      [8, 0],
    ]);
    const w = writeRange(pts, 2, 4, [], { ids });
    expect(w.remove).toEqual(["p1"]);
    const out = apply(pts, w);
    for (const [t, v] of [
      [1, 0.25],
      [2, 0.5],
      [4, 0.5],
      [6, 0.5],
      [7, 0.25],
      [8, 0],
    ] as const)
      expect(valueAt(out, t)).toBeCloseTo(v, 12);
  });

  it("slice then write over the same range round-trips", () => {
    const pts = lane([
      [0, 0],
      [8, 1],
    ]);
    const piece = sliceRange(pts, 2, 6)!;
    expect(piece.length).toBe(4);
    expect(piece.points.map((p) => p.value)).toEqual([0.25, 0.75]);
    const out = apply(pts, writeRange(pts, 2, 4, piece.points, { ids }));
    for (const t of [0, 1, 2, 3, 5, 6, 7, 9]) expect(valueAt(out, t)).toBeCloseTo(valueAt(pts, t)!, 12);
  });

  it("duplicate: the range is repeated right after itself, the rest kept", () => {
    // A ramp up 0..4 then flat at 1 until a point at 12.
    const pts = lane([
      [0, 0],
      [4, 1],
      [12, 0],
    ]);
    const piece = sliceRange(pts, 0, 4)!;
    const out = apply(pts, writeRange(pts, 4, 4, piece.points, { ids }));
    // The copy ramps 0 → 1 over 4..8 (a jump at 4), then the original curve continues.
    expect(valueBefore(out, 4)).toBeCloseTo(1, 12);
    expect(valueAt(out, 4)).toBeCloseTo(0, 12);
    expect(valueAt(out, 6)).toBeCloseTo(0.5, 12);
    expect(valueAt(out, 10)).toBeCloseTo(valueAt(pts, 10)!, 12);
    expect(valueAt(out, 12)).toBe(0);
  });

  it("into an empty lane: only the piece (no edges)", () => {
    const piece = sliceRange(
      lane([
        [0, 0.3],
        [2, 0.6],
      ]),
      0,
      2,
    )!;
    const w = writeRange([], 10, 2, piece.points, { ids, mapValue: (v) => v / 2 });
    expect(w.remove).toEqual([]);
    expect(w.add.map((p) => [p.time, p.value])).toEqual([
      [10, 0.15],
      [12, 0.3],
    ]);
    const command = rangeWriteCommand(
      w,
      { lane: null, owner: { type: "Track", track: "t" }, target: { type: "TrackPan", track: "t" }, newLaneId: "L2" },
      keyedStep("undo.automation.paste_range", "paste automation"),
    );
    expect(command).toMatchObject({
      domain: "Edit",
      command: {
        type: "Batch",
        label: "paste automation",
        key: { id: "undo.automation.paste_range", args: {} },
        commands: [{ command: { type: "CreateLane", id: "L2" } }, { command: { type: "AddPoints", lane: "L2" } }],
      },
    });
  });

  it("nothing to do: no command", () => {
    expect(
      rangeWriteCommand(
        { remove: [], add: [] },
        { lane: null, owner: { type: "Track", track: "t" }, target: { type: "TrackPan", track: "t" } },
        keyedStep("undo.automation.paste_range", "x"),
      ),
    ).toBeNull();
    expect(sliceRange([], 0, 4)).toBeNull();
  });
});

describe("sectionMovePlan: moving a time section up/down, the outside unchanged", () => {
  const byTimeThenId = (a: AutomationPoint, b: AutomationPoint) => a.time - b.time || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
  const curved = (pts: Array<[number, number, CurveShape]>): AutomationPoint[] =>
    pts.map(([time, value, curve], i) => ({ id: `p${i}`, lane: "L", time, value, curve }));
  /** The lane after the plan's points are added and the section moved by `dv`. */
  function moved(pts: AutomationPoint[], plan: SectionPlan, dv: number): { lane: AutomationPoint[]; edits: PointEdit[] } {
    const edits = moveEdits(plan.moving, plan.moving[0]!, 0, dv, null, { lockTime: true });
    const all = [...pts, ...plan.add.map((p) => ({ ...p, lane: "L" }))].map((p) => {
      const e = edits.find((x) => x.id === p.id);
      return e ? { ...p, value: e.value ?? p.value } : p;
    });
    return { lane: all.sort(byTimeThenId), edits };
  }
  const samples = (from: number, to: number) => Array.from({ length: 401 }, (_, k) => from + ((to - from) * k) / 400);
  const diff = (a: AutomationPoint[], b: AutomationPoint[], t: number) => Math.abs(evaluatePoints(a, t)! - evaluatePoints(b, t)!);

  it("a linear ramp: edge jumps added, inside shifted by the delta, outside unchanged", () => {
    // 0.2 → 0.8 over 16 beats, section [4, 12].
    const pts = lane([
      [0, 0.2],
      [16, 0.8],
    ]);
    const plan = sectionMovePlan(pts, 4, 12, "L", ids)!;
    expect(plan.add.map((p) => [p.time, p.value])).toEqual([
      [4, expect.closeTo(0.35, 12)],
      [4, expect.closeTo(0.35, 12)],
      [12, expect.closeTo(0.65, 12)],
      [12, expect.closeTo(0.65, 12)],
    ]);
    // Outer first at the start, inner first at the end (id order).
    expect(plan.moving.map((p) => p.id)).toEqual([plan.add[1]!.id, plan.add[2]!.id]);
    const { lane: after } = moved(pts, plan, 0.1);
    for (const t of [...samples(0, 3.999), ...samples(12.001, 20)]) expect(diff(after, pts, t)).toBeLessThan(1e-12);
    for (const t of samples(4, 11.999)) expect(evaluatePoints(after, t)).toBeCloseTo(evaluatePoints(pts, t)! + 0.1, 12);
    // The original points are untouched.
    expect(after.find((p) => p.id === "p0")).toEqual(pts[0]);
    expect(after.find((p) => p.id === "p1")).toEqual(pts[1]);
  });

  it("points inside move; step and linear segments keep their shape outside", () => {
    const pts = curved([
      [0, 0.1, { type: "Step" }],
      [6, 0.5, { type: "Curve", tension: 0.6 }],
      [10, 0.9, { type: "Linear" }],
      [14, 0.3, { type: "Curve", tension: -0.4 }],
      [20, 0.6, { type: "Linear" }],
    ]);
    const plan = sectionMovePlan(pts, 4, 12, "L", ids)!;
    expect(plan.moving.map((p) => p.id)).toEqual([plan.add[1]!.id, "p1", "p2", plan.add[2]!.id]);
    const { lane: after } = moved(pts, plan, -0.05);
    // Before the section (a step), after it (the rest of a linear segment, then the
    // untouched points): exact.
    for (const t of [...samples(-2, 3.999), ...samples(12.001, 24)]) expect(diff(after, pts, t)).toBeLessThan(1e-12);
    // Inside: shifted (the 4 → 6 part of the step holds, the curve 6 → 10 is unchanged).
    for (const t of samples(4, 11.999)) expect(evaluatePoints(after, t)).toBeCloseTo(evaluatePoints(pts, t)! - 0.05, 12);
    for (const id of ["p0", "p3", "p4"]) expect(after.find((p) => p.id === id)).toEqual(pts.find((p) => p.id === id));
  });

  it("a curve cut by an edge: exact before it, the closest tension after it", () => {
    const pts = curved([
      [0, 0.1, { type: "Curve", tension: 0.5 }],
      [16, 0.9, { type: "Linear" }],
    ]);
    const { lane: after } = moved(pts, sectionMovePlan(pts, 4, 8, "L", ids)!, 0.05);
    for (const t of samples(0, 3.999)) expect(diff(after, pts, t)).toBeLessThan(1e-12);
    for (const t of samples(8.001, 16)) expect(diff(after, pts, t)).toBeLessThan(0.0225 * 0.8);
  });

  it("reuses points already at the edges: one there gets a partner, a jump is used as is", () => {
    const pts = lane([
      [0, 0.2],
      [4, 0.4],
      [12, 0.6],
      [16, 0.8],
    ]);
    const plan = sectionMovePlan(pts, 4, 12, "L", ids)!;
    // The point at the start edge stays outside; a new inner one follows it (a later id). At
    // the end edge the existing point is the inner one (it sorts first) and a new one keeps
    // its value outside.
    expect(plan.add).toHaveLength(2);
    expect(plan.add[0]).toMatchObject({ time: 4, value: 0.4 });
    expect(plan.add[1]).toMatchObject({ time: 12, value: 0.6, curve: { type: "Linear" } });
    expect(plan.moving.map((p) => p.id)).toEqual([plan.add[0]!.id, "p2"]);
    const { lane: after } = moved(pts, plan, 0.2);
    for (const t of [...samples(0, 3.999), ...samples(12, 20)]) expect(diff(after, pts, t)).toBeLessThan(1e-12);
    for (const t of samples(4, 11.999)) expect(evaluatePoints(after, t)).toBeCloseTo(evaluatePoints(pts, t)! + 0.2, 12);

    // Jumps already at both edges: nothing added.
    const jumps = lane([
      [4, 0.3],
      [4, 0.5],
      [12, 0.5],
      [12, 0.3],
    ]);
    const plan2 = sectionMovePlan(jumps, 4, 12, "L", ids)!;
    expect(plan2.add).toEqual([]);
    expect(plan2.moving.map((p) => p.id)).toEqual(["p1", "p2"]);
  });

  it("clamps the section as a group (like a multi-point drag)", () => {
    const pts = lane([
      [0, 0.2],
      [8, 0.9],
      [16, 0.2],
    ]);
    const { lane: after } = moved(pts, sectionMovePlan(pts, 4, 12, "L", ids)!, 0.5);
    // The highest inside point (0.9) stops at 1: everything inside moved by 0.1.
    expect(after.find((p) => p.id === "p1")!.value).toBeCloseTo(1, 12);
    expect(evaluatePoints(after, 4)).toBeCloseTo(0.55 + 0.1, 12);
    expect(evaluatePoints(after, 0)).toBe(0.2);
    expect(evaluatePoints(after, 16)).toBe(0.2);
  });

  it("before the first and after the last point, and the no-op cases", () => {
    const pts = lane([[6, 0.4]]);
    const { lane: after } = moved(pts, sectionMovePlan(pts, 2, 4, "L", ids)!, 0.3);
    expect(evaluatePoints(after, 1)).toBe(0.4);
    expect(evaluatePoints(after, 3)).toBeCloseTo(0.7, 12);
    expect(evaluatePoints(after, 5)).toBe(0.4);
    expect(sectionMovePlan([], 0, 4, "L", ids)).toBeNull();
    expect(sectionMovePlan(pts, 4, 4, "L", ids)).toBeNull();
  });
});
