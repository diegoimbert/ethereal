import { describe, expect, it } from "vitest";
import type { AutomationPoint, ParamInfo } from "@/generated";
import { paramToNormalized, paramToPlain } from "@/features/devices/paramScale";
import { moveEdits } from "./edit";
import { PAN_INFO, VOLUME_INFO } from "./params";
import {
  clampRange,
  defaultRange,
  dragHint,
  FULL_RANGE,
  incrementPlain,
  nudgeSize,
  paramStep,
  rangeValueToY,
  rangeYToValue,
  scrollRange,
  snapValue,
  stepCount,
  stepDragDelta,
  stepLines,
  zoomRange,
} from "./valueAxis";

const param = (over: Partial<ParamInfo>): ParamInfo => ({
  id: 1,
  name: "P",
  group: null,
  unit: "None",
  min: 0,
  max: 1,
  default: 0,
  scale: { type: "Linear" },
  labels: null,
  automatable: true,
  hidden: false,
  ...over,
});

const TRANSPOSE = param({ name: "Transpose", unit: "Semitones", min: -24, max: 24, default: 0 });
const WIDE = param({ name: "Transpose", unit: "Semitones", min: -48, max: 48, default: 0 });
const WAVE = param({ name: "Waveform", min: 0, max: 3, default: 1, labels: ["Sine", "Saw", "Square", "Triangle"] });
const TOGGLE = param({ name: "Bypass", unit: "Toggle" });
const PERCENT = param({ name: "Mix", unit: "Percent", min: 0, max: 100, default: 50 });
const CUTOFF = param({ name: "Cutoff", unit: "Hertz", min: 20, max: 20000, default: 1000, scale: { type: "Log" } });

const plain = (info: ParamInfo, n: number) => paramToPlain(info, n);

describe("steps", () => {
  it("infers stepped params", () => {
    expect(paramStep(TRANSPOSE)).toBe(1);
    expect(stepCount(TRANSPOSE)).toBe(48);
    expect(paramStep(WAVE)).toBe(1);
    expect(stepCount(WAVE)).toBe(3);
    expect(stepCount(TOGGLE)).toBe(1);
    expect(paramStep(PERCENT)).toBeNull();
    expect(paramStep(VOLUME_INFO)).toBeNull();
    expect(paramStep({ ...PERCENT, step: 5 } as ParamInfo)).toBe(5);
    // With the field present, `step: null` is continuous: no semitone inference.
    expect(paramStep({ ...TRANSPOSE, step: null } as ParamInfo)).toBeNull();
    expect(paramStep({ ...TRANSPOSE, step: 1 } as ParamInfo)).toBe(1);
    expect(paramStep({ ...WAVE, step: null } as ParamInfo)).toBe(1);
  });

  it("stepped params always snap to whole steps", () => {
    const n = paramToNormalized(TRANSPOSE, 2.4);
    expect(plain(TRANSPOSE, snapValue(TRANSPOSE, n))).toBeCloseTo(2, 9);
    expect(plain(TRANSPOSE, snapValue(TRANSPOSE, paramToNormalized(TRANSPOSE, -0.6)))).toBeCloseTo(-1, 9);
    expect(plain(WAVE, snapValue(WAVE, 0.55))).toBe(2);
    expect(snapValue(TOGGLE, 0.7)).toBe(1);
    expect(snapValue(TOGGLE, 0.3)).toBe(0);
  });

  it("continuous params are free unless the step modifier is held", () => {
    expect(snapValue(PERCENT, 0.123)).toBe(0.123);
    expect(plain(PERCENT, snapValue(PERCENT, 0.123, true))).toBeCloseTo(12, 9);
    const v = snapValue(VOLUME_INFO, 0.8, true);
    expect(plain(VOLUME_INFO, v)).toBeCloseTo(Math.round(plain(VOLUME_INFO, 0.8)), 6);
    expect(plain(PAN_INFO, snapValue(PAN_INFO, 0.6234, true))).toBeCloseTo(0.25, 9);
    expect(plain(CUTOFF, snapValue(CUTOFF, paramToNormalized(CUTOFF, 1234), true))).toBeCloseTo(1200, 6);
  });

  it("increments", () => {
    expect(incrementPlain(VOLUME_INFO, -6.4)).toBe(-6);
    expect(incrementPlain(CUTOFF, 447)).toBe(450);
    expect(incrementPlain(param({ min: 0, max: 10 }), 3.337)).toBeCloseTo(3.3, 9);
  });

  it("nudges one step, or 1 % (shift: 0.1 %)", () => {
    expect(nudgeSize(TRANSPOSE)).toBeCloseTo(1 / 48, 12);
    expect(nudgeSize(TRANSPOSE, true)).toBeCloseTo(1 / 48, 12);
    expect(nudgeSize(PERCENT)).toBe(0.01);
    expect(nudgeSize(PERCENT, true)).toBe(0.001);
  });

  it("hints the modifiers", () => {
    expect(dragHint(TRANSPOSE)).toBe("⌥ off grid · ⇧ one axis");
    expect(dragHint(VOLUME_INFO)).toBe("⌘ 1 dB steps · ⌥ off grid · ⇧ one axis");
    expect(dragHint(VOLUME_INFO, "Ctrl")).toMatch(/^Ctrl 1 dB steps/);
  });

  it("drags of a stepped param land on steps (the group follows the anchor)", () => {
    const pts: AutomationPoint[] = [
      { id: "a", lane: "L", time: 0, value: paramToNormalized(TRANSPOSE, 0), curve: { type: "Linear" } },
      { id: "b", lane: "L", time: 1, value: paramToNormalized(TRANSPOSE, 5), curve: { type: "Linear" } },
    ];
    const edits = moveEdits(pts, pts[0]!, 0, 0.051, null, { snapValue: (v) => snapValue(TRANSPOSE, v) });
    expect(plain(TRANSPOSE, edits[0]!.value!)).toBeCloseTo(2, 9);
    expect(plain(TRANSPOSE, edits[1]!.value!)).toBeCloseTo(7, 9);
  });
});

describe("visible range", () => {
  it("stepped params open on a window with steps at least 8 px tall, around the default", () => {
    // Default lane: 64 px - 2 × 5 px padding = 54 px → 6 steps (9 px each): ±3 st.
    const r = defaultRange(TRANSPOSE, 54);
    expect(plain(TRANSPOSE, r.lo)).toBeCloseTo(-3, 9);
    expect(plain(TRANSPOSE, r.hi)).toBeCloseTo(3, 9);
    expect(54 / ((r.hi - r.lo) * 48)).toBeGreaterThanOrEqual(8);
    // A taller lane shows more; a lane with room for every step shows them all.
    const tall = defaultRange(WIDE, 400);
    expect(plain(WIDE, tall.lo)).toBeCloseTo(-25, 9);
    expect(plain(WIDE, tall.hi)).toBeCloseTo(25, 9);
    expect(defaultRange(TRANSPOSE, 400)).toEqual(FULL_RANGE);
    expect(defaultRange(WAVE, 54)).toEqual(FULL_RANGE);
    expect(defaultRange(PERCENT, 54)).toEqual(FULL_RANGE);
    // Centred on a given value, slid back inside the range at the ends.
    const top = defaultRange(TRANSPOSE, 54, 1);
    expect(plain(TRANSPOSE, top.hi)).toBeCloseTo(24, 9);
    expect(plain(TRANSPOSE, top.lo)).toBeCloseTo(18, 9);
  });

  it("stepped drags move whole steps at 8 px per step, whatever the lane height", () => {
    expect(stepDragDelta(TRANSPOSE, 24)! * 48).toBeCloseTo(3, 12);
    expect(stepDragDelta(TRANSPOSE, 27)! * 48).toBeCloseTo(3, 12);
    expect(stepDragDelta(TRANSPOSE, -9)! * 48).toBeCloseTo(-1, 12);
    expect(stepDragDelta(TRANSPOSE, 3)).toBe(0);
    expect(stepDragDelta(PERCENT, 24)).toBeNull();
  });

  it("scrolls and zooms inside 0..1", () => {
    const r = { lo: 0.25, hi: 0.75 };
    expect(scrollRange(r, 0.5)).toEqual({ lo: 0.5, hi: 1 });
    expect(scrollRange(r, -1)).toEqual({ lo: 0, hi: 0.5 });
    const z = zoomRange(r, 0.5, 0.5);
    expect(z.lo).toBeCloseTo(0.375, 12);
    expect(z.hi).toBeCloseTo(0.625, 12);
    expect(zoomRange(r, 10, 0.5)).toEqual(FULL_RANGE);
    // No narrower than a few steps.
    const tiny = clampRange({ lo: 0.5, hi: 0.5001 }, TRANSPOSE);
    expect(tiny.hi - tiny.lo).toBeCloseTo(4 / 48, 12);
  });

  it("maps values through the window", () => {
    const r = { lo: 0.5, hi: 1 };
    expect(rangeValueToY(1, 110, 5, r)).toBe(5);
    expect(rangeValueToY(0.5, 110, 5, r)).toBe(105);
    expect(rangeYToValue(55, 110, 5, r)).toBeCloseTo(0.75, 12);
    // Below the window: still a value (clamped to 0..1 only).
    expect(rangeYToValue(155, 110, 5, r)).toBeCloseTo(0.25, 12);
    expect(rangeYToValue(500, 110, 5, r)).toBe(0);
  });
});

describe("step lines", () => {
  it("one line per step when there is room, labelled where it fits", () => {
    const lines = stepLines(WAVE, FULL_RANGE, 54);
    expect(lines.map((l) => plain(WAVE, l.value))).toEqual([0, 1, 2, 3]);
    expect(lines.map((l) => l.label)).toEqual(["Sine", "Saw", "Square", "Triangle"]);
  });

  it("thins dense steps and marks the default", () => {
    const lines = stepLines(TRANSPOSE, FULL_RANGE, 54);
    // 48 steps in 54 px: every 6th semitone (≥ 6 px apart).
    expect(lines.map((l) => plain(TRANSPOSE, l.value))).toEqual([-24, -18, -12, -6, 0, 6, 12, 18, 24]);
    expect(lines.find((l) => l.major)!.value).toBeCloseTo(0.5, 12);
    // Taller lane: every semitone.
    expect(stepLines(TRANSPOSE, FULL_RANGE, 300)).toHaveLength(49);
    expect(stepLines(PERCENT, FULL_RANGE, 300)).toEqual([]);
  });

  it("only inside the window", () => {
    const r = { lo: paramToNormalized(TRANSPOSE, -2), hi: paramToNormalized(TRANSPOSE, 2) };
    const lines = stepLines(TRANSPOSE, r, 60);
    expect(lines.map((l) => Math.round(plain(TRANSPOSE, l.value)))).toEqual([-2, -1, 0, 1, 2]);
    expect(lines.every((l) => l.label !== null)).toBe(true);
  });
});
