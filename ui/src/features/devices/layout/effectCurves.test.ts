import { describe, expect, it } from "vitest";
import {
  compressorReductionDb,
  delayTaps,
  divisionBeats,
  dynamicsModeOf,
  dynamicsOutDb,
  reverbEnvelope,
  saturatorCurve,
  saturatorTransfer,
  velocityMap,
  type DynamicsShape,
} from "./effectCurves";

const base: DynamicsShape = { mode: "compress", threshold: -20, ratio: 4, knee: 6, floor: 0, inputGain: 0 };

describe("effect curves", () => {
  it("compressor gain computer mirrors the device (soft knee, ratio)", () => {
    expect(compressorReductionDb(-40, -20, 4, 6)).toBe(0);
    expect(compressorReductionDb(-8, -20, 4, 6)).toBeCloseTo(9, 6);
    // Continuous at the knee edges; hard knee is exact.
    expect(compressorReductionDb(-23, -20, 4, 6)).toBeCloseTo(0, 9);
    expect(compressorReductionDb(-17, -20, 4, 6)).toBeCloseTo(2.25, 9);
    expect(compressorReductionDb(-10, -20, 4, 0)).toBeCloseTo(7.5, 9);
  });

  it("transfer curves per mode", () => {
    expect(dynamicsOutDb(base, -8)).toBeCloseTo(-17, 6);
    expect(dynamicsOutDb({ ...base, mode: "limit", threshold: -1 }, -10)).toBe(-10);
    expect(dynamicsOutDb({ ...base, mode: "limit", threshold: -1, inputGain: 6 }, -3)).toBe(-1);
    // Expander: 2 dB under at 3:1 → 4 dB more reduction (capped by the floor).
    expect(dynamicsOutDb({ ...base, mode: "expand", ratio: 3 }, -22)).toBeCloseTo(-26, 6);
    expect(dynamicsOutDb({ ...base, mode: "expand", ratio: 20, floor: -10 }, -40)).toBeCloseTo(-50, 6);
    expect(dynamicsOutDb({ ...base, mode: "gate", floor: -80 }, -30)).toBe(-110);
    expect(dynamicsOutDb({ ...base, mode: "gate", floor: -80 }, -10)).toBe(-10);
    expect(dynamicsModeOf("Expand", true)).toBe("expand");
    expect(dynamicsModeOf("Expander", true)).toBe("expand");
    expect(dynamicsModeOf("Gate", true)).toBe("gate");
    expect(dynamicsModeOf("RMS", true)).toBe("compress");
    expect(dynamicsModeOf(null, false)).toBe("limit");
  });

  it("delay taps decay by the feedback, freeze holds them", () => {
    const taps = delayTaps(0.25, 0.5, false, 2);
    expect(taps.map((t) => t.t)).toEqual([0.25, 0.5, 0.75, 1, 1.25, 1.5, 1.75, 2]);
    expect(taps[2]!.level).toBeCloseTo(0.25, 9);
    expect(taps.map((t) => t.side).slice(0, 3)).toEqual([0, 1, 0]);
    expect(delayTaps(0.25, 0, false, 2)).toHaveLength(1);
    expect(delayTaps(0.5, 0.2, true, 2).every((t) => t.level === 1)).toBe(true);
    expect(divisionBeats("1/8 D")).toBeCloseTo(0.75, 9);
    expect(divisionBeats("1/4 T")).toBeCloseTo(2 / 3, 9);
    expect(divisionBeats("1/1")).toBe(4);
    expect(divisionBeats("bar")).toBeNull();
  });

  it("reverb envelope: pre-delay gap, -60 dB at the decay time, frozen = flat", () => {
    expect(reverbEnvelope(0.01, 0.02, 2)).toBe(0);
    expect(reverbEnvelope(0.02, 0.02, 2)).toBe(1);
    expect(reverbEnvelope(2.02, 0.02, 2)).toBeCloseTo(0.001, 9);
    expect(reverbEnvelope(30, 0, Infinity)).toBe(1);
  });

  it("velocity map mirrors the device", () => {
    const flat = { drive: 0, compand: 0, inLow: 1, inHigh: 127, outLow: 1, outHigh: 127, mode: "clip" as const };
    expect(velocityMap(flat, 64)).toBeCloseTo(64, 9);
    expect(velocityMap({ ...flat, drive: 100 }, 33)!).toBeGreaterThan(33);
    expect(velocityMap({ ...flat, drive: -100 }, 33)!).toBeLessThan(33);
    expect(velocityMap({ ...flat, mode: "fixed", outHigh: 100 }, 5)).toBe(100);
    expect(velocityMap({ ...flat, mode: "gate", inLow: 40 }, 20)).toBeNull();
    expect(velocityMap({ ...flat, outLow: 60, outHigh: 80 }, 127)).toBe(80);
  });

  it("saturator curves by label", () => {
    const tube = saturatorCurve("Tube")!;
    expect(tube(1)).toBeCloseTo(1 - Math.exp(-1), 9);
    expect(saturatorCurve("Rectify")!(-2)).toBeGreaterThan(0);
    expect(saturatorCurve("Wavetable")).toBeNull();
    const f = saturatorTransfer(saturatorCurve("Hard")!, 4, 0);
    expect(f(1)).toBe(1);
    expect(f(0)).toBe(0);
  });
});
