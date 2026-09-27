import { afterEach, describe, expect, it } from "vitest";
import { playheadStore } from "@/state/playhead";

const frame = (position: number) => ({
  transport: { position, seconds: position / 2, playing: true, bpm: 120 },
});

afterEach(() => playheadStore.reset());

describe("playhead override while listening", () => {
  it("ignores engine frames until cleared", () => {
    let calls = 0;
    const off = playheadStore.subscribePlayhead(() => calls++);
    playheadStore.setPlayhead(frame(1));
    playheadStore.setOverride(frame(10));
    playheadStore.setPlayhead(frame(2)); // the local engine: ignored
    expect(playheadStore.getPlayhead()?.transport.position).toBe(10);
    playheadStore.setOverride(null);
    expect(playheadStore.getPlayhead()?.transport.position).toBe(10); // until the engine's next frame
    playheadStore.setPlayhead(frame(3));
    expect(playheadStore.getPlayhead()?.transport.position).toBe(3);
    expect(calls).toBe(3);
    off();
  });
});
