import { describe, expect, it } from "vitest";
import type { PeerTransport } from "@/generated";
import { TempoMap } from "@/timeline";
import { extrapolate, HOLD_AFTER_MS, SampleTracker } from "./extrapolate";

const t = (p: Partial<PeerTransport>): PeerTransport => ({ position: 0, playing: true, sent_at_ms: 1, loop_region: null, ...p });
const at120 = TempoMap.constant(120);

describe("peer playhead extrapolation", () => {
  it("holds a stopped playhead", () => {
    expect(extrapolate({ transport: t({ position: 5, playing: false }), arrivedAt: 0 }, 10_000, at120)).toBe(5);
  });

  it("advances from the arrival time over the tempo map", () => {
    // 120 bpm: 2 beats per second.
    expect(extrapolate({ transport: t({ position: 4 }), arrivedAt: 1000 }, 1500, at120)).toBeCloseTo(5);
    // A tempo change at beat 8 (to 60 bpm): 2 beats to reach it (1 s), then 1 beat per second.
    const map = new TempoMap(
      [
        { id: "a", time: 0, bpm: 120, curve: "Step" },
        { id: "b", time: 8, bpm: 60, curve: "Step" },
      ],
      [{ id: "s", time: 0, signature: { numerator: 4, denominator: 4 } }],
    );
    expect(extrapolate({ transport: t({ position: 6 }), arrivedAt: 0 }, 1500, map)).toBeCloseTo(8.5);
  });

  it("wraps into the loop when the sample was inside it", () => {
    const loop = { start: 4, end: 8 };
    // 1.5 s = 3 beats from 6 → 9 → wrapped to 5.
    expect(extrapolate({ transport: t({ position: 6, loop_region: loop }), arrivedAt: 0 }, 1500, at120)).toBeCloseTo(5);
    // Outside the loop: not wrapped.
    expect(extrapolate({ transport: t({ position: 1, loop_region: loop }), arrivedAt: 0 }, 1500, at120)).toBeCloseTo(4);
  });

  it("holds when refreshes stop arriving", () => {
    const s = { transport: t({ position: 0 }), arrivedAt: 0 };
    const held = extrapolate(s, HOLD_AFTER_MS, at120);
    expect(extrapolate(s, HOLD_AFTER_MS + 60_000, at120)).toBe(held);
  });

  it("records arrival on each new sample", () => {
    const tr = new SampleTracker();
    tr.update([{ site: "2", state: { transport: t({ sent_at_ms: 1 }) } }], 100);
    tr.update([{ site: "2", state: { transport: t({ sent_at_ms: 1 }) } }], 900);
    expect(tr.get("2")!.arrivedAt).toBe(100);
    tr.update([{ site: "2", state: { transport: t({ sent_at_ms: 2 }) } }], 1000);
    expect(tr.get("2")!.arrivedAt).toBe(1000);
    tr.update([{ site: "2", state: {} }], 1100);
    expect(tr.get("2")).toBeUndefined();
  });
});
