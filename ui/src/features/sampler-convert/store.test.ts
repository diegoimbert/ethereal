import { describe, expect, it } from "vitest";
import { DEFAULT_KEY_SPAN, presetKeys, presetVelocities, sampleCount, sampleNoteName, sortedSet, velocityToAdd } from "./store";

describe("convert to sampler", () => {
  it("key presets step over the picked keys' span, the highest included", () => {
    expect(presetKeys(2, [60, 64])).toEqual([60, 62, 64]);
    expect(presetKeys(2, [65, 60, 61])).toEqual([60, 62, 64, 65]);
    expect(presetKeys(12, [])).toEqual([36, 48, 60, 72, 84, 96]);
    // Fewer than two keys: C1–C6.
    expect(presetKeys(1, [70])).toHaveLength(DEFAULT_KEY_SPAN[1] - DEFAULT_KEY_SPAN[0] + 1);
  });

  it("velocity presets go evenly up to 127", () => {
    expect(presetVelocities(1)).toEqual([127]);
    expect(presetVelocities(3)).toEqual([42, 85, 127]);
    expect(presetVelocities(16)).toHaveLength(16);
    expect(new Set(presetVelocities(16)).size).toBe(16);
  });

  it("adds a velocity in the widest gap", () => {
    expect(velocityToAdd([127])).toBe(63);
    expect(velocityToAdd([42, 85, 127])).toBe(63);
    expect(velocityToAdd([1, 127])).toBe(64);
    expect(velocityToAdd(Array.from({ length: 127 }, (_, i) => i + 1))).toBeNull();
  });

  it("counts samples, duplicates once", () => {
    expect(sampleCount({ keys: [60, 62, 64], velocities: [127] })).toBe(3);
    expect(sampleCount({ keys: [60, 62, 64, 60], velocities: [42, 85, 127, 85] })).toBe(9);
    expect(sampleCount({ keys: [], velocities: [127] })).toBe(0);
    expect(sortedSet([3, 1, 3])).toEqual([1, 3]);
  });

  it("names notes like the rest of the app (C3 = 60)", () => {
    expect(sampleNoteName(60)).toBe("c3");
    expect(sampleNoteName(61)).toBe("c#3");
    expect(sampleNoteName(36)).toBe("c1");
  });
});
