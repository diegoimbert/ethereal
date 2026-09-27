/** Web host anchor math (docs/COLLAB.md §9.4 "Host math (web)"). */
import { describe, expect, it } from "vitest";
import {
  anchor,
  barsToBeats,
  CountInTracker,
  ENCODER_DELAY_S,
  encodedFrameStart,
  renderContextTime,
  RtpOffsetEstimator,
  rtpDiff,
  u32,
} from "./anchors";
import { HEADER, readTapClock, tapClockBuffer, TapClockWriter, type TapState } from "./tapClock";

const TRANSPORT = { loop_enabled: true, loop_region: { start: 0, end: 8 }, metronome: false };
const state = (s: Partial<TapState>): TapState => ({
  tapFrame: 0,
  position: 0,
  playing: true,
  recording: false,
  bpm: 120,
  latency: 0,
  ...s,
});

describe("wrapping RTP arithmetic", () => {
  it("wraps into u32 and compares as i32", () => {
    expect(u32(-1)).toBe(2 ** 32 - 1);
    expect(u32(2 ** 32 + 5)).toBe(5);
    expect(rtpDiff(5, 2 ** 32 - 5)).toBe(10);
    expect(rtpDiff(2 ** 32 - 5, 5)).toBe(-10);
    expect(rtpDiff(1_000_480, 1_000_000)).toBe(480);
  });
});

describe("performance time → context frames", () => {
  it("maps an instant to the frame being rendered, minus the encoder delay", () => {
    const ts = { contextTime: 10, performanceTime: 5_000 };
    // 100 ms later the device plays context time 10.1; the render is 20 ms ahead of it.
    expect(renderContextTime(5_100, ts, 0.02)).toBeCloseTo(10.12, 9);
    expect(encodedFrameStart(5_100, ts, 0.02, 48_000)).toBeCloseTo((10.12 - ENCODER_DELAY_S) * 48_000, 6);
  });
});

describe("RtpOffsetEstimator", () => {
  it("recovers the RTP offset at 44.1 kHz and ignores delayed observations", () => {
    const sr = 44_100;
    const offset = 123_456_789;
    const e = new RtpOffsetEstimator(sr, 9);
    expect(e.offset()).toBeNull();
    expect(e.rtpAt(0)).toBeNull();
    for (let k = 0; k < 9; k++) {
      const frame = 1_000_000 + k * 882; // one 20 ms Opus frame = 882 frames at 44.1 kHz
      const rtp = u32(offset + Math.round((frame * 48_000) / sr));
      // Three observations were late (the page was busy): they look like later frames.
      const late = k % 3 === 0 ? 2_000 : 0;
      e.observe(rtp, frame + late);
    }
    expect(e.offset()).toBe(offset);
    expect(e.rtpAt(sr)).toBe(offset + 48_000);
  });

  it("wraps across 2^32", () => {
    const e = new RtpOffsetEstimator(48_000);
    const offset = 2 ** 32 - 100;
    for (let k = 0; k < 5; k++) e.observe(u32(offset + k * 960), k * 960);
    expect(e.offset()).toBe(offset);
    expect(e.rtpAt(1_000)).toBe(900);
  });
});

describe("anchors", () => {
  it("anchors a loop wrap when its first sample is heard (worked example of §9.4)", () => {
    // 48 kHz, L = 480 samples, 120 bpm, loop 0..8; the wrap is rendered at context frame
    // T, where rtp(T) = 1_000_000.
    const T = 5_000_000;
    const e = new RtpOffsetEstimator(48_000);
    e.observe(1_000_000, T);
    const L = 480;
    // Worklet: tapFrame = render frame + latency.
    const before = state({ tapFrame: T - 4_800 + L, position: 7.8, latency: L });
    const wrap = state({ tapFrame: T + L, position: 0, latency: L });
    const a1 = anchor(before, e.rtpAt(before.tapFrame)!, TRANSPORT, false, null);
    const a2 = anchor(wrap, e.rtpAt(wrap.tapFrame)!, TRANSPORT, true, null);
    expect(a1).toMatchObject({ rtp: 995_680, position: 7.8, discontinuity: false, loop_enabled: true });
    expect(a2).toMatchObject({ rtp: 1_000_480, position: 0, discontinuity: true });
    expect(a2).not.toHaveProperty("count_in_end");
    // A listener playing r = 1_000_300 still maps with the first anchor: just before 8.
    const r = 1_000_300;
    expect(rtpDiff(r, a2.rtp)).toBeLessThan(0);
    expect(a1.position + (rtpDiff(r, a1.rtp) / 48_000) * 2).toBeCloseTo(7.9925, 9);
  });

  it("anchors wrap across 2^32", () => {
    const e = new RtpOffsetEstimator(48_000);
    e.observe(2 ** 32 - 240, 1_000);
    const a = anchor(state({ tapFrame: 1_000 + 480 }), e.rtpAt(1_480)!, TRANSPORT, true, null);
    expect(a.rtp).toBe(240);
  });

  it("carries count_in_end when given", () => {
    const a = anchor(state({ recording: true, position: -4 }), 1, TRANSPORT, false, 0);
    expect(a.count_in_end).toBe(0);
    expect(a.recording).toBe(true);
  });
});

describe("CountInTracker", () => {
  it("sets the record start when a recording starts from stopped, clears at stop", () => {
    const t = new CountInTracker();
    const preRoll = barsToBeats(1, { numerator: 3, denominator: 4 });
    expect(preRoll).toBe(3);
    expect(t.update(state({ playing: false }), preRoll)).toBeNull();
    // Armed, located to the pre-roll, playing: count-in until 16.
    expect(t.update(state({ playing: true, recording: true, position: 13 }), preRoll)).toBe(16);
    expect(t.update(state({ playing: true, recording: true, position: 15 }), preRoll)).toBe(16);
    expect(t.update(state({ playing: false, recording: false }), preRoll)).toBeNull();
    // Punch-in while playing: no count-in.
    expect(t.update(state({ playing: true, position: 20 }), preRoll)).toBeNull();
    expect(t.update(state({ playing: true, recording: true, position: 21 }), preRoll)).toBeNull();
    // No count-in bars.
    const u = new CountInTracker();
    u.update(state({ playing: false }), 0);
    expect(u.update(state({ playing: true, recording: true }), 0)).toBeNull();
  });
});

describe("tap clock", () => {
  const header = (h: Partial<Record<keyof typeof HEADER | "JUMP_POSITION" | "JUMP_PLAYING" | "JUMP_LATENCY", number>>) => {
    const f = new Float64Array(HEADER.LEN);
    f[HEADER.BLOCKS] = 1;
    f[HEADER.PLAYING] = 1;
    f[HEADER.BPM] = 120;
    for (const [k, v] of Object.entries(h)) {
      const idx = { JUMP_POSITION: 9, JUMP_PLAYING: 10, JUMP_LATENCY: 13 }[k] ?? HEADER[k as keyof typeof HEADER];
      f[idx] = v;
    }
    return f;
  };

  it("publishes tap frames (render frame + offset + latency) and counts discontinuities", () => {
    const buf = tapClockBuffer();
    expect(readTapClock(buf)).toBeNull();
    const w = new TapClockWriter(buf, 48_000);
    // First block: a fresh timeline is an event.
    w.write(1_280, header({ POSITION: 1, LATENCY: 64 }));
    let c = readTapClock(buf)!;
    expect(c.sampleRate).toBe(48_000);
    expect(c.latest).toEqual({ tapFrame: 1_344, position: 1, playing: true, recording: false, bpm: 120, latency: 64 });
    expect(c.events).toBe(1);

    // Nothing tapped: unchanged.
    w.write(1_408, header({ BLOCKS: 0, POSITION: 99 }));
    expect(readTapClock(buf)!.latest.position).toBe(1);

    // Steady: no event.
    w.write(1_408, header({ POSITION: 1.01, LATENCY: 64 }));
    expect(readTapClock(buf)!.events).toBe(1);

    // A loop wrap in the middle of the block (offset 32): the event is the jump sub-block.
    w.write(1_536, header({ POSITION: 0.02, OFFSET: 64, LATENCY: 64, JUMPED: 1, JUMP_OFFSET: 32, JUMP_POSITION: 0, JUMP_PLAYING: 1, JUMP_LATENCY: 64 }));
    c = readTapClock(buf)!;
    expect(c.events).toBe(2);
    expect(c.event).toMatchObject({ tapFrame: 1_536 + 32 + 64, position: 0, playing: true });
    expect(c.latest).toMatchObject({ tapFrame: 1_536 + 64 + 64, position: 0.02 });

    // Stop (not a jump) and a latency change are events too.
    w.write(1_664, header({ PLAYING: 0, POSITION: 0.1, LATENCY: 64 }));
    expect(readTapClock(buf)!.events).toBe(3);
    expect(readTapClock(buf)!.event.playing).toBe(false);
    w.write(1_792, header({ PLAYING: 0, POSITION: 0.1, LATENCY: 128 }));
    expect(readTapClock(buf)!.events).toBe(4);
  });
});
