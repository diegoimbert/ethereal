import { describe, expect, it } from "vitest";
import { beatsToPx, pxToBeats, type TimelineViewport } from ".";

describe("timeline mapping", () => {
  const vp: TimelineViewport = { pxPerBeat: 20, scrollBeats: 4 };

  it("maps beats to pixels relative to the scroll position", () => {
    expect(beatsToPx(4, vp)).toBe(0);
    expect(beatsToPx(8, vp)).toBe(80);
    expect(beatsToPx(2, vp)).toBe(-40);
  });

  it("pxToBeats inverts beatsToPx", () => {
    for (const b of [0, 1.5, 4, 17.25]) expect(pxToBeats(beatsToPx(b, vp), vp)).toBeCloseTo(b);
  });
});
