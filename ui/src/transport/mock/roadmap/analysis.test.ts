/** MockTransport: `Analysis` watches (v0.2, fx-analysis). */
import { describe, expect, it } from "vitest";
import { MockAnalysis } from "./analysis";

describe("MockAnalysis", () => {
  it("tracks watched devices", () => {
    const a = new MockAnalysis();
    expect(a.command({ type: "Watch", device: "d1" })).toEqual({ type: "Unit" });
    expect(a.watched.has("d1")).toBe(true);
    a.command({ type: "Unwatch", device: "d1" });
    expect(a.watched.size).toBe(0);
  });
});
