import { describe, expect, it } from "vitest";
import { adaptiveStep, formatGridStep, gridLines, resolveGrid, snapDelta, snapToGrid } from "./grid";
import { TempoMap } from "./tempoMap";

const four = { numerator: 4, denominator: 4 };
const m44 = TempoMap.constant(120);

describe("adaptive grid", () => {
  it("picks the finest step at least minPx apart", () => {
    expect(adaptiveStep(100, 24, four)).toEqual({ kind: "beats", beats: 0.25 });
    expect(adaptiveStep(20, 24, four)).toEqual({ kind: "beats", beats: 2 });
    expect(adaptiveStep(5, 24, four)).toEqual({ kind: "bars", bars: 2 });
    expect(adaptiveStep(0.5, 24, four)).toEqual({ kind: "bars", bars: 16 });
  });

  it("triplet steps are 2/3 of straight ones", () => {
    const s = adaptiveStep(100, 24, four, true);
    expect(s.kind).toBe("beats");
    if (s.kind === "beats") expect(s.beats).toBeCloseTo(1 / 3);
  });

  it("resolveGrid handles off and fixed", () => {
    expect(resolveGrid({ type: "Off" }, 10, four)).toBeNull();
    expect(resolveGrid({ type: "Fixed", step: { kind: "beats", beats: 0.5 }, triplet: false }, 10, four)).toEqual({
      kind: "beats",
      beats: 0.5,
    });
  });
});

describe("gridLines", () => {
  it("marks bars, beats and subdivisions", () => {
    const lines = gridLines(m44, { start: 0, end: 4.1 }, { kind: "beats", beats: 0.5 });
    expect(lines.map((l) => [l.beats, l.level])).toEqual([
      [0, "bar"],
      [0.5, "sub"],
      [1, "beat"],
      [1.5, "sub"],
      [2, "beat"],
      [2.5, "sub"],
      [3, "beat"],
      [3.5, "sub"],
      [4, "bar"],
    ]);
  });

  it("starts mid-bar and restarts subdivisions at each bar line (7/8)", () => {
    const m = new TempoMap([{ id: "t", time: 0, bpm: 120, curve: "Step" }], [
      { id: "s", time: 0, signature: { numerator: 7, denominator: 8 } },
    ]);
    const lines = gridLines(m, { start: 2.5, end: 4.5 }, { kind: "beats", beats: 1 });
    expect(lines.map((l) => [l.beats, l.level])).toEqual([
      [3, "beat"],
      [3.5, "bar"],
    ]);
  });

  it("bar steps", () => {
    const lines = gridLines(m44, { start: 0, end: 32 }, { kind: "bars", bars: 2 });
    expect(lines.map((l) => l.bar)).toEqual([1, 3, 5, 7]);
  });
});

describe("snapToGrid", () => {
  it("snaps to the nearest sub-bar line, with floor/ceil modes", () => {
    const step = { kind: "beats", beats: 0.25 } as const;
    expect(snapToGrid(1.1, step, m44)).toBeCloseTo(1);
    expect(snapToGrid(1.2, step, m44)).toBeCloseTo(1.25);
    expect(snapToGrid(1.2, step, m44, "floor")).toBeCloseTo(1);
    expect(snapToGrid(1.01, step, m44, "ceil")).toBeCloseTo(1.25);
    // Float error near a line stays on it.
    expect(snapToGrid(1.25 + 1e-9, step, m44, "ceil")).toBeCloseTo(1.25);
  });

  it("snaps relative to bar lines in odd meters", () => {
    const m = new TempoMap([{ id: "t", time: 0, bpm: 120, curve: "Step" }], [
      { id: "s", time: 0, signature: { numerator: 7, denominator: 8 } },
    ]);
    // Bar 2 starts at 3.5; 1-beat grid within it: 3.5, 4.5, 5.5 ...
    expect(snapToGrid(4.4, { kind: "beats", beats: 1 }, m)).toBeCloseTo(4.5);
    // Bar end is a valid target (3.4 → 3.5, not 4).
    expect(snapToGrid(3.4, { kind: "beats", beats: 1 }, m)).toBeCloseTo(3.5);
  });

  it("snaps to every N bars", () => {
    const step = { kind: "bars", bars: 2 } as const;
    expect(snapToGrid(5, step, m44)).toBe(8);
    expect(snapToGrid(3, step, m44)).toBe(0);
    expect(snapToGrid(9, step, m44, "floor")).toBe(8);
    expect(snapToGrid(9, step, m44, "ceil")).toBe(16);
    expect(snapToGrid(8, step, m44, "ceil")).toBe(8);
  });

  it("grid off returns the input", () => {
    expect(snapToGrid(1.234, null, m44)).toBe(1.234);
  });

  it("snapDelta: absolute and relative", () => {
    const step = { kind: "beats", beats: 1 } as const;
    expect(snapDelta(0.3, 1.1, step, m44)).toBeCloseTo(0.7);
    expect(snapDelta(0.3, 1.1, step, m44, true)).toBeCloseTo(1);
  });
});

describe("formatGridStep", () => {
  it("names steps", () => {
    expect(formatGridStep({ kind: "beats", beats: 0.25 })).toBe("1/16");
    expect(formatGridStep({ kind: "beats", beats: 1 / 3 })).toBe("1/8T");
    expect(formatGridStep({ kind: "bars", bars: 1 })).toBe("1 Bar");
    expect(formatGridStep({ kind: "bars", bars: 4 })).toBe("4 Bars");
    expect(formatGridStep(null)).toBe("Off");
  });
});
