import { describe, expect, it } from "vitest";
import { beatsApproxEq, ceilBeats, floorBeats, snapBeats } from "./beats";

// Same vectors as the Rust `beats_tests` in crates/ether-model/src/value.rs.
describe("beats helpers", () => {
  it("absorb float error", () => {
    expect(beatsApproxEq((1 / 3) * 3, 1)).toBe(true);
    expect(beatsApproxEq(1, 1.00001)).toBe(false);
    expect(snapBeats(0.26, 0.25)).toBe(0.25);
    expect(floorBeats(0.9999999, 1)).toBe(1);
    expect(ceilBeats(1.0000001, 1)).toBe(1);
    expect(ceilBeats(1.3, 1)).toBe(2);
  });

  it("return x unchanged for grid <= 0", () => {
    for (const f of [snapBeats, floorBeats, ceilBeats]) {
      expect(f(1.234, 0)).toBe(1.234);
      expect(f(1.234, -1)).toBe(1.234);
    }
  });
});
