// The tempo lane's multi-point edits (pure).
import { describe, expect, it } from "vitest";
import type { Command, TempoPoint } from "@/generated";
import { clipOfPoints, clipOfRange, deletePointsCommand, deleteRangeCommand, movePointsCommand, pasteCommand, pointsIn, setCurvesCommand } from "./tempoEdits";

const pt = (id: string, time: number, bpm: number, curve: TempoPoint["curve"] = "Step"): TempoPoint => ({ id, time, bpm, curve });
const P = [pt("z", 0, 120), pt("a", 4, 140), pt("b", 8, 100, "Linear"), pt("c", 12, 90)];

/** The tempo sub-commands of `c` (unwrapping a batch). */
const subs = (c: Command | null) => {
  if (!c) return [];
  const list = c.domain === "Edit" && c.command.type === "Batch" ? c.command.commands : [c];
  return list.map((x) => (x.domain === "Tempo" ? x.command : x));
};

describe("tempo lane edits", () => {
  it("deleting never removes the point at beat 0", () => {
    expect(subs(deletePointsCommand(P, ["z", "a", "c"]))).toEqual([{ type: "RemoveTempoPoints", ids: ["a", "c"] }]);
    expect(deletePointsCommand(P, ["z"])).toBeNull();
    expect(pointsIn(P, 4, 12).map((p) => p.id)).toEqual(["a", "b"]);
    expect(subs(deleteRangeCommand(P, 0, 9))).toEqual([{ type: "RemoveTempoPoints", ids: ["a", "b"] }]);
  });

  it("moves a selection as one command, in an order with no collision; refuses landing on another point or at 0", () => {
    const right = subs(movePointsCommand(P, new Set(["a", "b"]), 4, 0));
    // b (8 → 12) would hit c: refused.
    expect(right).toEqual([]);
    const ok = subs(movePointsCommand(P, new Set(["a", "b"]), 2, 5));
    expect(ok.map((c) => ("id" in c ? c.id : null))).toEqual(["b", "a"]); // rightmost first
    expect(ok[0]).toMatchObject({ type: "EditTempoPoint", id: "b", time: 10, bpm: 105 });
    expect(movePointsCommand(P, new Set(["a"]), -4, 0)).toBeNull(); // onto beat 0
    // The point at 0 keeps its time, only its BPM changes.
    expect(subs(movePointsCommand(P, new Set(["z", "a"]), 1, -10))).toEqual([
      { type: "EditTempoPoint", id: "a", time: 5, bpm: 130, curve: null },
      { type: "EditTempoPoint", id: "z", time: null, bpm: 110, curve: null },
    ]);
  });

  it("sets the curve of the selection, skipping points already so", () => {
    expect(subs(setCurvesCommand(P, ["a", "b"], "Linear"))).toEqual([{ type: "EditTempoPoint", id: "a", time: null, bpm: null, curve: "Linear" }]);
  });

  it("copies points relative to the first, and a section with the tempo in effect at its start", () => {
    expect(clipOfPoints(P, new Set(["b", "a"]))).toEqual({
      points: [
        { dt: 0, bpm: 140, curve: "Step" },
        { dt: 4, bpm: 100, curve: "Linear" },
      ],
      span: 4,
      range: false,
    });
    expect(clipOfRange(P, 6, 10)).toEqual({
      points: [
        { dt: 0, bpm: 140, curve: "Step" },
        { dt: 2, bpm: 100, curve: "Linear" },
      ],
      span: 4,
      range: true,
    });
  });

  it("pastes: adds points, takes over one already there, and a section replaces its span", () => {
    let n = 0;
    const mint = () => `new${n++}`;
    const points = pasteCommand(P, clipOfPoints(P, new Set(["a", "b"]))!, 8, mint);
    // 8 is taken (b): b takes a's BPM; 12 is taken (c): c takes b's.
    expect(points.ids).toEqual(["b", "c"]);
    expect(subs(points.command)).toEqual([
      { type: "EditTempoPoint", id: "b", time: null, bpm: 140, curve: "Step" },
      { type: "EditTempoPoint", id: "c", time: null, bpm: 100, curve: "Linear" },
    ]);
    const section = pasteCommand(P, clipOfRange(P, 2, 6)!, 7, mint);
    // The span 7..11 is replaced: b (8) goes; the section's start tempo (120) and a's (140 at +2).
    expect(subs(section.command)).toEqual([
      { type: "RemoveTempoPoints", ids: ["b"] },
      { type: "AddTempoPoint", id: "new0", time: 7, bpm: 120, curve: "Step" },
      { type: "AddTempoPoint", id: "new1", time: 9, bpm: 140, curve: "Step" },
    ]);
    expect(section.ids).toEqual(["new0", "new1"]);
  });
});
