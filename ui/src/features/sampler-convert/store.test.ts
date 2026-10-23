import { describe, expect, it } from "vitest";
import { MAX_CONVERT_SAMPLES, sampleCount, sampledKeyCount, sampleNoteName } from "./store";

describe("convert to sampler", () => {
  it("counts the keys the engine samples, including the last", () => {
    expect(sampledKeyCount(60, 64, 2)).toBe(3); // 60, 62, 64
    expect(sampledKeyCount(60, 65, 2)).toBe(4); // 60, 62, 64, 65
    expect(sampledKeyCount(60, 64, 1)).toBe(5);
    expect(sampledKeyCount(60, 64, 10)).toBe(2);
    expect(sampledKeyCount(64, 60, 1)).toBe(0);
  });

  it("counts samples per velocity layer and caps the dialog", () => {
    expect(sampleCount({ firstKey: 60, lastKey: 64, keyStep: 2, velocitySteps: 1 })).toBe(3);
    expect(sampleCount({ firstKey: 60, lastKey: 64, keyStep: 2, velocitySteps: 3 })).toBe(9);
    expect(sampleCount({ firstKey: 0, lastKey: 127, keyStep: 1, velocitySteps: 5 })).toBeGreaterThan(MAX_CONVERT_SAMPLES);
  });

  it("names notes like the rest of the app (C3 = 60)", () => {
    expect(sampleNoteName(60)).toBe("c3");
    expect(sampleNoteName(61)).toBe("c#3");
    expect(sampleNoteName(36)).toBe("c1");
  });
});
