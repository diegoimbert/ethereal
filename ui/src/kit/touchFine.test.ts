import { describe, expect, it } from "vitest";
import { touchFineFactor } from "./touchFine";

describe("touchFineFactor", () => {
  it("full rate near the axis, slower the further the finger moves off it, either side", () => {
    expect(touchFineFactor(0)).toBe(1);
    expect(touchFineFactor(59)).toBe(1);
    expect(touchFineFactor(60)).toBe(0.5);
    expect(touchFineFactor(-130)).toBe(0.25);
    expect(touchFineFactor(180)).toBe(0.1);
    expect(touchFineFactor(900)).toBe(0.1);
  });
});
