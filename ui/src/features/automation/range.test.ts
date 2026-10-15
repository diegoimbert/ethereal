import { describe, expect, it } from "vitest";
import type { AutomationPoint, PointSpec } from "@/generated";
import { rangeWriteCommand, sliceRange, valueAt, valueBefore, writeRange, type RangePoint } from "./range";
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
