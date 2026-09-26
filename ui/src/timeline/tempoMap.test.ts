import { describe, expect, it } from "vitest";
import type { TempoPoint, TimeSignaturePoint } from "@/generated";
import { beatsPerBar, TempoMap } from "./tempoMap";

const tp = (time: number, bpm: number, curve: TempoPoint["curve"] = "Step"): TempoPoint => ({
  id: `t${time}`,
  time,
  bpm,
  curve,
});
const sig = (time: number, numerator: number, denominator: number): TimeSignaturePoint => ({
  id: `s${time}`,
  time,
  signature: { numerator, denominator },
});

describe("TempoMap", () => {
  it("constant tempo: 120 BPM = 0.5 s per beat", () => {
    const m = TempoMap.constant(120);
    expect(m.beatsToSeconds(0)).toBe(0);
    expect(m.beatsToSeconds(4)).toBeCloseTo(2);
    expect(m.secondsToBeats(2)).toBeCloseTo(4);
    expect(m.bpmAt(100)).toBe(120);
  });

  it("step tempo changes integrate per segment", () => {
    const m = new TempoMap([tp(8, 60), tp(0, 120)], [sig(0, 4, 4)]);
    expect(m.beatsToSeconds(8)).toBeCloseTo(4);
    expect(m.beatsToSeconds(10)).toBeCloseTo(6);
    expect(m.secondsToBeats(6)).toBeCloseTo(10);
    expect(m.bpmAt(7.9)).toBe(120);
    expect(m.bpmAt(8)).toBe(60);
  });

  it("linear ramps: bpm interpolates and seconds invert", () => {
    const m = new TempoMap([tp(0, 60, "Linear"), tp(4, 120)], [sig(0, 4, 4)]);
    expect(m.bpmAt(2)).toBeCloseTo(90);
    // ∫0..4 60/(60+15x) dx = 4 ln 2
    expect(m.beatsToSeconds(4)).toBeCloseTo(4 * Math.log(2));
    for (const b of [0, 1, 2.5, 4, 7]) expect(m.secondsToBeats(m.beatsToSeconds(b))).toBeCloseTo(b);
    expect(m.bpmAt(6)).toBe(120);
  });

  it("falls back to 120 BPM / 4/4 when empty", () => {
    const m = new TempoMap([], []);
    expect(m.bpmAt(0)).toBe(120);
    expect(m.signatureAt(0)).toEqual({ numerator: 4, denominator: 4 });
  });

  it("beatsPerBar", () => {
    expect(beatsPerBar({ numerator: 4, denominator: 4 })).toBe(4);
    expect(beatsPerBar({ numerator: 6, denominator: 8 })).toBe(3);
    expect(beatsPerBar({ numerator: 7, denominator: 8 })).toBe(3.5);
  });

  it("bar/beat positions are 1-based and follow signature changes", () => {
    // 2 bars of 4/4 (0..8), then 3/4.
    const m = new TempoMap([tp(0, 120)], [sig(0, 4, 4), sig(8, 3, 4)]);
    expect(m.barBeat(0)).toEqual({ bar: 1, beat: 1, fraction: 0 });
    expect(m.barBeat(5.5)).toEqual({ bar: 2, beat: 2, fraction: 0.5 });
    expect(m.barBeat(8)).toEqual({ bar: 3, beat: 1, fraction: 0 });
    expect(m.barBeat(11)).toEqual({ bar: 4, beat: 1, fraction: 0 });
    expect(m.barToBeats(4)).toBe(11);
    expect(m.barToBeats(2)).toBe(4);
    // Float error just below a bar line counts as on it.
    expect(m.barAt(11 - 1e-9).bar).toBe(4);
  });

  it("6/8 beats are eighth notes", () => {
    const m = new TempoMap([tp(0, 120)], [sig(0, 6, 8)]);
    expect(m.barBeat(3)).toEqual({ bar: 2, beat: 1, fraction: 0 });
    expect(m.barBeat(4)).toEqual({ bar: 2, beat: 3, fraction: 0 });
  });

  it("barLines enumerates bars, optionally every N", () => {
    const m = new TempoMap([tp(0, 120)], [sig(0, 4, 4), sig(8, 3, 4)]);
    expect(m.barLines(0, 15).map((b) => [b.bar, b.beats])).toEqual([
      [1, 0],
      [2, 4],
      [3, 8],
      [4, 11],
      [5, 14],
    ]);
    expect(m.barLines(1, 15, 2).map((b) => b.bar)).toEqual([3, 5]);
  });
});
