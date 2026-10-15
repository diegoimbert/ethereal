import { describe, expect, it } from "vitest";
import { beatRects, type BeatLayout } from "./beatCanvas";

const layout = (over: Partial<BeatLayout> = {}): BeatLayout => ({
  rows: [36, 38],
  rowH: 20,
  visible: [0, 1, 2, 3, 4, 5],
  steps: 4,
  xOf: (i) => i * 10,
  cellW: 10,
  hitAt: (note, i) => (note === 36 && i === 1 ? 1 : null),
  looping: false,
  alt: () => false,
  bar: (i) => i === 0 || i === 4,
  hover: null,
  insetX: 1,
  insetY: 2,
  radius: 4,
  barWidth: 1,
  ...over,
});
const colors = { text: [1, 1, 1, 1] as const, accent: [1, 0, 0, 1] as const, bar: [0.5, 0.5, 0.5, 1] as const };

describe("beatRects", () => {
  it("one pad per visible step and row, inset in its slot, plus bar lines", () => {
    const rects = beatRects(layout(), colors);
    const pads = rects.filter((r) => r.radius > 0);
    expect(pads).toHaveLength(2 * 6);
    expect(pads[0]).toMatchObject({ x: 1, y: 2, w: 8, h: 16 });
    // Rows stack by row height.
    expect(pads[6]).toMatchObject({ x: 1, y: 22 });
    expect(rects.filter((r) => r.radius === 0)).toHaveLength(2 * 2);
  });

  it("hits in the accent; past the clip's end everything fades, hits repeat only when looping", () => {
    const red = (rs: ReturnType<typeof beatRects>) => rs.filter((r) => r.radius > 0 && r.color[1] === 0).map((r) => [r.x, r.color[3]]);
    expect(red(beatRects(layout(), colors))).toEqual([[11, 1]]);
    // Step 5 = step 1 of the second pass: drawn at 30 % when the clip loops.
    expect(red(beatRects(layout({ looping: true }), colors))).toEqual([
      [11, 1],
      [51, 0.3],
    ]);
  });
});
