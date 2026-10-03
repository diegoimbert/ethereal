import { describe, expect, it } from "vitest";
import { zoomAnchorPx } from "./useTimelineWheel";

describe("zoomAnchorPx", () => {
  it("anchors on the pointer over the timeline", () => {
    expect(zoomAnchorPx(500, 200)).toBe(300);
  });
  it("clamps to the timeline's left edge over the left pane", () => {
    expect(zoomAnchorPx(120, 200)).toBe(0);
    expect(zoomAnchorPx(0, 200)).toBe(0);
  });
});
