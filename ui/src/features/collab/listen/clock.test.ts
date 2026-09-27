import { describe, expect, it } from "vitest";
import type { StreamClock } from "@/generated";
import { TempoMap } from "@/timeline/tempoMap";
import { MAX_ANCHORS, playoutRtp, rtpAdd, rtpDiff, StreamClockMapper } from "./clock";

function anchor(rtp: number, position: number, extra: Partial<StreamClock> = {}): StreamClock {
  return {
    rtp: rtp >>> 0,
    position,
    playing: true,
    recording: false,
    bpm: 120,
    loop_enabled: false,
    loop_region: { start: 0, end: 8 },
    metronome: false,
    discontinuity: false,
    ...extra,
  };
}

describe("rtp arithmetic", () => {
  it("compares wrapping u32 timestamps as i32 differences", () => {
    expect(rtpDiff(10, 5)).toBe(5);
    expect(rtpDiff(5, 10)).toBe(-5);
    expect(rtpDiff(3, 0xffff_fffe)).toBe(5);
    expect(rtpDiff(0xffff_fffe, 3)).toBe(-5);
    expect(rtpAdd(0xffff_ffff, 2)).toBe(1);
    expect(rtpAdd(1, -2)).toBe(0xffff_ffff);
  });

  it("extrapolates the playout timestamp and subtracts the output latency", () => {
    // Delivered 100 ms ago, 10 ms of output latency: 90 ms = 4320 samples later.
    expect(playoutRtp({ rtpTimestamp: 1000, timestamp: 5_000 }, 5_100, 0.01)).toBe(1000 + 4320);
    expect(playoutRtp({ rtpTimestamp: 0xffff_ff00, timestamp: 0 }, 10, 0)).toBe((0xffff_ff00 + 480) >>> 0);
    expect(playoutRtp({ timestamp: 0 }, 10, 0)).toBeNull();
  });
});

describe("StreamClockMapper", () => {
  it("maps the worked example of COLLAB.md §9.4 (latency-shifted loop wrap)", () => {
    const m = new StreamClockMapper();
    const loop = { loop_enabled: true, loop_region: { start: 0, end: 8 } };
    m.push(anchor(995_680, 7.8, loop));
    m.push(anchor(1_000_480, 0, { ...loop, discontinuity: true }));
    // Still hearing the pre-wrap timeline (in the PDC delay line).
    expect(m.map(1_000_300)!.position).toBeCloseTo(7.9925, 6);
    // From the wrap anchor's rtp on: 0.
    expect(m.map(1_000_480)!.position).toBe(0);
    expect(m.map(1_000_480 + 4800)!.position).toBeCloseTo(0.2, 9);
  });

  it("predicts the loop wrap before the wrap anchor arrives", () => {
    const m = new StreamClockMapper();
    m.push(anchor(0, 7.9, { loop_enabled: true, loop_region: { start: 4, end: 8 } }));
    // 0.1 s at 120 bpm = 0.2 beats → 8.1 → wraps to 4.1.
    expect(m.map(4800)!.position).toBeCloseTo(4.1, 9);
  });

  it("never interpolates across a discontinuity and holds a stopped position", () => {
    const m = new StreamClockMapper();
    m.push(anchor(0, 2));
    m.push(anchor(48_000, 16, { discontinuity: true })); // locate
    expect(m.map(47_999)!.position).toBeCloseTo(2 + (47_999 / 48_000) * 2, 6);
    expect(m.map(48_000)!.position).toBe(16);
    m.push(anchor(96_000, 18, { playing: false }));
    const stopped = m.map(200_000)!;
    expect(stopped.playing).toBe(false);
    expect(stopped.position).toBe(18);
  });

  it("works across the u32 wrap", () => {
    const m = new StreamClockMapper();
    const base = 2 ** 32 - 2400; // wraps 50 ms after this anchor
    m.push(anchor(base, 1));
    m.push(anchor(rtpAdd(base, 48_000), 3)); // pushed after the wrap
    expect(m.size).toBe(2);
    // 25 ms after the wrap: 75 ms after the first anchor.
    expect(m.map(1200)!.position).toBeCloseTo(1 + 0.075 * 2, 9);
    expect(m.map(rtpAdd(base, 48_000 + 24_000))!.position).toBeCloseTo(4, 9);
  });

  it("orders anchors that arrive out of order and drops the ones before the anchor in use", () => {
    const m = new StreamClockMapper();
    m.push(anchor(9600, 0.4));
    m.push(anchor(0, 0));
    m.push(anchor(4800, 0.2));
    expect(m.map(4800 + 2400)!.anchor.rtp).toBe(4800);
    expect(m.size).toBe(2);
    for (let i = 0; i < MAX_ANCHORS + 10; i++) m.push(anchor(20_000 + i * 4800, 1 + i * 0.2));
    expect(m.size).toBe(MAX_ANCHORS);
  });

  it("holds the first anchor's position for audio heard before it", () => {
    const m = new StreamClockMapper();
    m.push(anchor(10_000, 4, { discontinuity: true }));
    expect(m.map(5_000)!.position).toBe(4);
  });

  it("uses the replicated tempo map, the anchor's bpm as a fallback", () => {
    // 60 bpm from beat 4 on: 1 s after beat 4 is beat 5 (not 6 at the anchor's 120).
    const tempo = new TempoMap(
      [
        { id: "a", time: 0, bpm: 120, curve: "Step" },
        { id: "b", time: 4, bpm: 60, curve: "Step" },
      ],
      [{ id: "s", time: 0, signature: { numerator: 4, denominator: 4 } }],
    );
    const m = new StreamClockMapper();
    m.push(anchor(0, 4));
    expect(m.map(48_000, tempo)!.position).toBeCloseTo(5, 6);
    expect(m.map(48_000)!.position).toBeCloseTo(6, 6);
  });

  it("maps count-in pre-roll (negative positions) and flags it", () => {
    const m = new StreamClockMapper();
    m.push(anchor(0, -4, { recording: true, count_in_end: 0, discontinuity: true }));
    const early = m.map(24_000)!; // 0.5 s = 1 beat
    expect(early.position).toBeCloseTo(-3, 9);
    expect(early.countIn).toBe(true);
    expect(early.recording).toBe(true);
    const late = m.map(96_000 + 4800)!; // 2.1 s = 4.2 beats
    expect(late.position).toBeCloseTo(0.2, 9);
    expect(late.countIn).toBe(false);
  });

  it("falls back to arrival time minus the receive delay", () => {
    const m = new StreamClockMapper();
    m.push(anchor(1000, 8), 10_000);
    // 300 ms later, 100 ms of receive delay: 0.2 s of audio = 0.4 beats.
    expect(m.mapByArrival(10_300, 0.1)!.position).toBeCloseTo(8.4, 6);
    expect(new StreamClockMapper().mapByArrival(0, 0)).toBeNull();
  });
});
