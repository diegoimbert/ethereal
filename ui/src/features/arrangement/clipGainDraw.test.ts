import { describe, expect, it } from "vitest";
import type { PeakData } from "@/generated";
import { CLIP_MARK_PX, dbToGain, drawWaveform, waveColumn, type DrawArea, type WaveformSource } from "./clipDraw";

/** Fake 2D context recording the waveform columns (`rect`) and clip marks (`fillRect`). */
function recorder() {
  const rects: Array<[number, number, number, number]> = [];
  const marks: Array<{ r: [number, number, number, number]; color: string }> = [];
  const ctx = {
    fillStyle: "",
    beginPath() {},
    fill() {},
    rect: (...r: [number, number, number, number]) => rects.push(r),
    fillRect(...r: [number, number, number, number]) {
      marks.push({ r, color: ctx.fillStyle });
    },
  };
  return { ctx: ctx as unknown as CanvasRenderingContext2D, rects, marks };
}

/** One second of constant ±`amp` peaks at 1000 Hz (1 frame per peak). */
function source(amp: number, gainDb?: number, reversed = false): WaveformSource {
  const n = 1000;
  const tile: PeakData = {
    media: "m",
    samples_per_peak: 1,
    start_frame: 0,
    min: [Array.from({ length: n }, () => -amp)],
    max: [Array.from({ length: n }, (_, i) => (reversed && i < n / 2 ? amp / 2 : amp))],
  };
  return {
    start: 0,
    clip: { length: 2, offset: 0, looping: { enabled: false, start: 0, end: 2 } },
    sampleRate: 1000,
    toSeconds: (c) => c / 2, // 2 beats per second
    frames: n,
    level: 1,
    gainDb,
    reversed,
    tile: (i) => (i === 0 ? tile : null),
  };
}

const AREA: DrawArea = { from: 0, to: 2, width: 100, height: 40 };
const height = (r: [number, number, number, number]) => r[3];

describe("clip gain waveform scaling", () => {
  it("converts dB to a linear factor; -inf is silence", () => {
    expect(dbToGain(0)).toBe(1);
    expect(dbToGain(6)).toBeCloseTo(1.995, 3);
    expect(dbToGain(-12)).toBeCloseTo(0.251, 3);
    expect(dbToGain(-144)).toBe(0);
    expect(dbToGain(-Infinity)).toBe(0);
    expect(dbToGain(Number.NaN)).toBe(0);
  });

  it("+6 dB roughly doubles the peak height", () => {
    const mid = 20;
    const base = waveColumn(-0.25, 0.25, dbToGain(0), mid);
    const up = waveColumn(-0.25, 0.25, dbToGain(6), mid);
    expect((up.bottom - up.top) / (base.bottom - base.top)).toBeCloseTo(2, 1);
    expect(up.clipTop || up.clipBottom).toBe(false);
  });

  it("clamps at full scale and flags the clipped side", () => {
    const col = waveColumn(-0.4, 0.8, dbToGain(6), 20);
    expect(col.top).toBe(0); // 0.8 × 2 = 1.6 → flattened at the lane edge
    expect(col.clipTop).toBe(true);
    expect(col.clipBottom).toBe(false); // -0.4 × 2 = -0.8 still fits
    expect(col.bottom).toBeCloseTo(20 + 0.8 * 20, 1);
  });

  it("-inf / silence draws a flat line", () => {
    const { ctx, rects } = recorder();
    const r = drawWaveform(ctx, AREA, source(0.9, -Infinity), "ink", "over");
    expect(rects.length).toBeGreaterThan(90);
    for (const rc of rects) {
      expect(height(rc)).toBe(1);
      expect(rc[1]).toBe(20);
    }
    expect(r.clipped).toBe(0);
  });

  it("draws taller, shorter and clipped waveforms from the same peaks", () => {
    const draw = (db: number) => {
      const rec = recorder();
      const res = drawWaveform(rec.ctx, AREA, source(0.6, db), "ink", "over");
      return { ...rec, res };
    };
    const zero = draw(0);
    const loud = draw(6);
    const quiet = draw(-12);
    expect(height(zero.rects[10]!)).toBeCloseTo(0.6 * 40, 5);
    expect(height(quiet.rects[10]!)).toBeCloseTo(0.6 * 0.251 * 40, 1);
    // 0.6 × 2 = 1.2: flattened to the full lane, marked at both edges in the clip color.
    expect(height(loud.rects[10]!)).toBe(40);
    expect(zero.res.clipped).toBe(0);
    expect(zero.marks).toHaveLength(0);
    expect(loud.res.clipped).toBe(loud.rects.length);
    expect(loud.marks.every((m) => m.color === "over" && m.r[3] === CLIP_MARK_PX)).toBe(true);
    expect(loud.marks.some((m) => m.r[1] === 0)).toBe(true);
    expect(loud.marks.some((m) => m.r[1] === 40 - CLIP_MARK_PX)).toBe(true);
  });

  it("composes with reverse: gain scales the mirrored columns", () => {
    // First half of the media is quieter; reversed, it is drawn on the right.
    const { ctx, rects } = recorder();
    drawWaveform(ctx, AREA, source(0.4, 6, true), "ink");
    const left = rects.find((r) => r[0] === 5)!;
    const right = rects.find((r) => r[0] === 95)!;
    expect(left[1]).toBeCloseTo(20 - 0.4 * 1.995 * 20, 1);
    expect(right[1]).toBeCloseTo(20 - 0.2 * 1.995 * 20, 1);
  });
});
