import { afterEach, describe, expect, it, vi } from "vitest";
import type { TrackMeter } from "@/generated";
import { playheadStore } from "./playhead";

const meter = (track: string, peak: number, clipped = false): TrackMeter => ({ track, peak: [peak, peak], rms: [peak / 2, peak / 2], clipped });

describe("playheadStore meters", () => {
  afterEach(() => playheadStore.reset());

  it("notifies a track's listeners only when its reading changes", () => {
    const a = vi.fn();
    const b = vi.fn();
    const offA = playheadStore.subscribeMeter("a", a);
    const offB = playheadStore.subscribeMeter("b", b);

    playheadStore.setMeters({ tracks: [meter("a", 0), meter("b", 0)], cpu_load: 0 });
    expect([a.mock.calls.length, b.mock.calls.length]).toEqual([1, 1]);
    const first = playheadStore.getMeter("a");

    // The engine resends every track each tick: a silent track stays put (same snapshot).
    playheadStore.setMeters({ tracks: [meter("a", 0), meter("b", 0.5)], cpu_load: 0 });
    expect([a.mock.calls.length, b.mock.calls.length]).toEqual([1, 2]);
    expect(playheadStore.getMeter("a")).toBe(first);
    expect(playheadStore.getMeter("b")?.peak).toEqual([0.5, 0.5]);

    playheadStore.setMeters({ tracks: [meter("a", 0, true)], cpu_load: 0 });
    expect(a).toHaveBeenCalledTimes(2);
    expect(playheadStore.getMeter("a")?.clipped).toBe(true);
    offA();
    offB();
  });
});
