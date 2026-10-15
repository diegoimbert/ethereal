import { describe, expect, it } from "vitest";
import { formatBarBeat, formatDuration, TempoMap } from "@/timeline";
import { parseLength, parsePosition } from "./position";
import { dragRangeEdge, rangeBox, rangeHandleAt } from "./rangeTouch";

describe("range handles", () => {
  const rows = [
    { track: { id: "a" }, y: 0, height: 60 },
    { track: { id: "b" }, y: 60, height: 60 },
    { track: { id: "c" }, y: 120, height: 60 },
  ];
  const vp = { pxPerBeat: 20, scrollBeats: 2 };

  it("the range's box: its rows, in lanes px", () => {
    expect(rangeBox({ start: 4, end: 8, tracks: ["b", "c"] }, rows, vp)).toEqual({ x0: 40, x1: 120, y0: 60, y1: 180 });
    expect(rangeBox({ start: 4, end: 8, tracks: ["z"] }, rows, vp)).toBeNull();
  });

  it("a handle is 44 px wide on its edge, over the range's rows only", () => {
    const box = { x0: 40, x1: 120, y0: 60, y1: 180 };
    expect(rangeHandleAt(box, 19, 100)).toBe("start");
    expect(rangeHandleAt(box, 62, 100)).toBe("start");
    expect(rangeHandleAt(box, 63, 100)).toBeNull();
    expect(rangeHandleAt(box, 141, 100)).toBe("end");
    expect(rangeHandleAt(box, 143, 100)).toBeNull();
    expect(rangeHandleAt(box, 40, 30)).toBeNull();
  });

  it("on a narrow range the nearer edge wins", () => {
    const box = { x0: 100, x1: 110, y0: 0, y1: 60 };
    expect(rangeHandleAt(box, 102, 10)).toBe("start");
    expect(rangeHandleAt(box, 108, 10)).toBe("end");
  });

  it("an end dragged past the other swaps them, never before 0", () => {
    expect(dragRangeEdge({ start: 4, end: 8 }, "end", 10)).toEqual({ start: 4, end: 10 });
    expect(dragRangeEdge({ start: 4, end: 8 }, "start", 10)).toEqual({ start: 8, end: 10 });
    expect(dragRangeEdge({ start: 4, end: 8 }, "start", -3)).toEqual({ start: 0, end: 8 });
  });
});

describe("typed positions and lengths", () => {
  const tempo = TempoMap.constant();

  it("bar.beat.sixteenth, as shown", () => {
    expect(parsePosition("1", tempo)).toBe(0);
    expect(parsePosition("2.2", tempo)).toBe(5);
    expect(parsePosition("2.2.2", tempo)).toBe(5.25);
    expect(parsePosition(" 3.1.1 ", tempo)).toBe(8);
    for (const b of [0, 5.25, 13.75]) expect(parsePosition(formatBarBeat(b, tempo), tempo)).toBe(b);
  });

  it("refuses what is not a position", () => {
    for (const t of ["", "0", "1.0", "1.5", "1.1.5", "a", "1..2", "1.2.3.4", "-1"]) expect(parsePosition(t, tempo)).toBeNull();
  });

  it("bars.beats.sixteenths lengths, as shown", () => {
    expect(parseLength("1", 4)).toBe(4);
    expect(parseLength("0.2", 4)).toBe(2);
    expect(parseLength("1.0.1", 4)).toBe(4.25);
    for (const l of [4, 2.5, 9.75]) expect(parseLength(formatDuration(l, 4), 4)).toBe(l);
    expect(parseLength("x", 4)).toBeNull();
  });
});
