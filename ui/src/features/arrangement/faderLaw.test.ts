import { describe, expect, it } from "vitest";
import { dbToFader, faderToDb } from "./faderLaw";

describe("fader law", () => {
  it("round-trips and pins 0 dB / -inf", () => {
    expect(faderToDb(dbToFader(0))).toBeCloseTo(0, 9);
    expect(faderToDb(dbToFader(-12))).toBeCloseTo(-12, 9);
    expect(faderToDb(0)).toBe(-144);
    expect(faderToDb(1)).toBeCloseTo(6, 9);
  });
});
