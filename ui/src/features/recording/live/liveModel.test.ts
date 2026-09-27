import { afterEach, describe, expect, it, vi } from "vitest";
import type { LiveAudioChunk, RecordingEvent } from "@/generated";
import { drawWaveform } from "@/features/arrangement/clipDraw";
import { peakRange, TILE_PEAKS } from "@/features/arrangement/peaks";
import { HANDOFF_GRACE_MS, HANDOFF_TIMEOUT_MS, liveEnd, LiveRecording, LiveTake, noteSpans, takeEnd } from "./liveModel";

const A = "01TRACKAAAAAAAAAAAAAAAAAAA";
const M = "01TRACKMMMMMMMMMMMMMMMMMMM";

function chunk(first: number, values: number[], take = 1): LiveAudioChunk {
  return {
    track: A,
    take,
    start: 4,
    sample_rate: 48_000,
    frames_per_peak: 256,
    first_peak: first as unknown as bigint,
    min: values.map((v) => -v),
    max: values,
  };
}

const progress = (audio: LiveAudioChunk[], midi: Extract<RecordingEvent, { type: "Progress" }>["midi"] = []): RecordingEvent => ({
  type: "Progress",
  audio,
  midi,
});

afterEach(() => vi.useRealTimers());

describe("live recording model", () => {
  it("accumulates peaks by first_peak, per take", () => {
    const m = new LiveRecording();
    m.apply({ type: "Started", tracks: [A] });
    m.apply(progress([chunk(0, [0.1, 0.2])]));
    m.apply(progress([chunk(2, [0.3]), chunk(0, [0.9], 2)]));
    // A dropped chunk leaves a gap that reads as silence.
    m.apply(progress([chunk(5, [0.5])]));
    const v = m.view(A);
    expect(v.phase).toBe("recording");
    expect(v.takes.map((t) => t.take)).toEqual([1, 2]);
    const t = v.takes[0]!;
    expect(t.count).toBe(6);
    expect([...t.max.subarray(0, 6)].map((x) => Math.round(x * 10) / 10)).toEqual([0.1, 0.2, 0.3, 0, 0, 0.5]);
    expect(t.frames).toBe(6 * 256);
    expect(t.start).toBe(4);
  });

  it("serves the peaks as tiles the waveform code reads", () => {
    const t = new LiveTake(A, 1, 0, 48_000, 256);
    const n = TILE_PEAKS + 10;
    const values = Array.from({ length: n }, (_, i) => (i % 7) / 10);
    t.put(0, values.slice(0, 600).map((v) => -v), values.slice(0, 600));
    const first = t.tile(0);
    t.put(600, values.slice(600).map((v) => -v), values.slice(600));
    // The partially filled tile was rebuilt, the new one exists.
    expect(t.tile(0)).not.toBe(first);
    expect(t.tile(0)!.max[0]!.length).toBe(TILE_PEAKS);
    expect(t.tile(1)!.max[0]!.length).toBe(10);
    expect(t.tile(2)).toBeNull();
    // Across the tile seam: frames of peaks 1020..1030.
    const r = peakRange(1020 * 256, 1030 * 256, 256, (i) => t.tile(i));
    expect(r!.max).toBeCloseTo(Math.max(...values.slice(1020, 1030)), 5);
    expect(r!.min).toBeCloseTo(-Math.max(...values.slice(1020, 1030)), 5);
  });

  it("draws the accumulated peaks with the arrangement waveform code", () => {
    const t = new LiveTake(A, 1, 2, 48_000, 256);
    // One beat at 120 bpm = 24000 frames ≈ 94 peaks: loud first half, silent second.
    const v = Array.from({ length: 94 }, (_, i) => (i < 47 ? 0.8 : 0));
    t.put(0, v.map((x) => -x), v);
    const rects: Array<[number, number, number, number]> = [];
    const ctx = { beginPath() {}, fill() {}, rect: (...r: [number, number, number, number]) => rects.push(r), fillStyle: "" };
    const len = takeEnd(t, 120) - t.start;
    expect(len).toBeCloseTo((94 * 256) / 24_000, 9);
    drawWaveform(
      ctx as unknown as CanvasRenderingContext2D,
      { from: 2, to: 3, width: 100, height: 20 },
      { start: 2, clip: { length: len, offset: 0, looping: { enabled: false, start: 0, end: len } }, sampleRate: 48_000, toSeconds: (c) => (c * 60) / 120, frames: t.frames, level: 256, tile: (i) => t.tile(i) },
      "#fff",
    );
    const tall = rects.filter((r) => r[3] > 10).map((r) => r[0]);
    expect(Math.min(...tall)).toBe(0);
    expect(Math.max(...tall)).toBeLessThan(52);
    expect(Math.max(...tall)).toBeGreaterThan(47);
  });

  it("pairs note starts and ends; held notes grow to the playhead", () => {
    const m = new LiveRecording();
    m.apply({ type: "Started", tracks: [M] });
    m.apply(progress([], [{ track: M, pitch: 60, velocity: 127, start: 1, length: null }]));
    expect(noteSpans(m.view(M).notes, 1.75)).toEqual([{ pitch: 60, velocity: 1, start: 1, end: 1.75 }]);
    m.apply(progress([], [{ track: M, pitch: 60, velocity: 127, start: 1, length: 0.5 }]));
    expect(m.view(M).notes).toEqual([{ pitch: 60, velocity: 127, start: 1, length: 0.5 }]);
    expect(noteSpans(m.view(M).notes, 3)[0]!.end).toBe(1.5);
  });

  it("grows to the playhead only while it can be the current take", () => {
    expect(liveEnd(4, 4.1, true, 0.5)).toBe(4.1);
    expect(liveEnd(4, 5, true, 0.5)).toBe(4); // punched out: data stopped
    expect(liveEnd(4, 1, true, 0.5)).toBe(4); // loop wrap: an earlier take
    expect(liveEnd(4, 4.1, false, 0.5)).toBe(4);
  });

  it("is replaced on Stopped once the committed peaks are ready", () => {
    vi.useFakeTimers();
    const m = new LiveRecording((clip) => (clip === "c1" ? "m1" : null));
    let notified = 0;
    m.subscribe(() => notified++);
    m.apply({ type: "Started", tracks: [A] });
    m.apply(progress([chunk(0, [0.5])]));
    m.apply({ type: "Stopped", clips: ["c1"] });
    // Frozen (not cleared) until the media's peaks are ready.
    expect(m.view(A).phase).toBe("handoff");
    expect(m.view(A).takes).toHaveLength(1);
    expect(m.apply(progress([chunk(1, [0.5])]))).toBe(false);
    vi.advanceTimersByTime(HANDOFF_GRACE_MS * 2);
    expect(m.view(A).takes).toHaveLength(1);
    m.peaksReady("m1");
    vi.advanceTimersByTime(HANDOFF_GRACE_MS);
    expect(m.view(A).takes).toHaveLength(0);
    expect(m.phase).toBe("idle");
    expect(notified).toBeGreaterThan(0);

    // Peaks already ready before Stopped (mock order), or never: grace / timeout.
    m.apply({ type: "Started", tracks: [A] });
    m.apply(progress([chunk(0, [0.5])]));
    m.peaksReady("m1");
    m.apply({ type: "Stopped", clips: ["c1"] });
    vi.advanceTimersByTime(HANDOFF_GRACE_MS);
    expect(m.view(A).takes).toHaveLength(0);
    m.apply({ type: "Started", tracks: [A] });
    m.apply(progress([chunk(0, [0.5])]));
    m.apply({ type: "Stopped", clips: ["c1"] });
    vi.advanceTimersByTime(HANDOFF_TIMEOUT_MS - 1);
    expect(m.view(A).takes).toHaveLength(1);
    vi.advanceTimersByTime(1);
    expect(m.view(A).takes).toHaveLength(0);
  });

  it("starts over on Started and ignores Progress outside a recording", () => {
    const m = new LiveRecording();
    expect(m.apply(progress([chunk(0, [0.5])]))).toBe(false);
    m.apply({ type: "Started", tracks: [A] });
    m.apply(progress([chunk(0, [0.5])]));
    const before = m.view(A);
    m.apply(progress([chunk(1, [0.5])]));
    expect(m.view(A)).not.toBe(before); // new snapshot per change
    m.apply({ type: "Started", tracks: [A] });
    expect(m.view(A).takes).toHaveLength(0);
  });
});
