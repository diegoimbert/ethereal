import { describe, expect, it } from "vitest";
import { fitPlacement } from "./placement";

const rect = (left: number, top: number, w = 40, h = 20) => new DOMRect(left, top, w, h);

describe("fitPlacement", () => {
  it("keeps the preferred placement when it fits", () => {
    expect(fitPlacement("bottom-start", rect(100, 100), 200, 150, 1000, 800)).toEqual({ placement: "bottom-start", dx: 0, maxHeight: undefined });
  });

  it("opens above when it overflows the bottom edge", () => {
    expect(fitPlacement("bottom-start", rect(100, 700), 200, 150, 1000, 800).placement).toBe("top-start");
  });

  it("aligns to the trigger's right edge near the right edge", () => {
    expect(fitPlacement("bottom-start", rect(900, 100), 200, 150, 1000, 800).placement).toBe("bottom-end");
  });

  it("nudges a panel wider than the room on both sides inside the window", () => {
    const fit = fitPlacement("bottom-start", rect(500, 100), 990, 150, 1000, 800);
    // Its right edge sits at the margin.
    expect(500 + fit.dx + 990).toBe(1000 - 4);
  });

  it("caps a panel taller than the room on both sides (it scrolls)", () => {
    const fit = fitPlacement("bottom-start", rect(100, 380), 200, 900, 1000, 800);
    expect(fit.placement).toBe("bottom-start");
    expect(fit.maxHeight).toBe(800 - 400 - 4);
  });
});
