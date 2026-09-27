import { describe, expect, it } from "vitest";
import { INTERP_DELAY_MS, PointerTrail } from "./interp";

const at = (beats: number, track: string | null = "a", y = 0) => ({ beats, track, y });

describe("PointerTrail", () => {
  it("draws one interval in the past, linearly between samples", () => {
    const t = new PointerTrail();
    expect(t.at(0)).toBeNull();
    t.push(at(0, "a", 0), 1000);
    t.push(at(10, "a", 1), 1040);
    expect(t.at(1000)).toEqual(at(0, "a", 0)); // before the first sample: hold it
    expect(t.at(1020 + INTERP_DELAY_MS)).toEqual(at(5, "a", 0.5));
    expect(t.at(1030 + INTERP_DELAY_MS)).toEqual(at(7.5, "a", 0.75));
    expect(t.at(2000)).toEqual(at(10, "a", 1)); // past the last: rest there
    expect(t.settled(1039 + INTERP_DELAY_MS)).toBe(false);
    expect(t.settled(1040 + INTERP_DELAY_MS)).toBe(true);
    expect(t.latest()).toEqual(at(10, "a", 1));
  });

  it("snaps on a track change instead of sliding across rows", () => {
    const t = new PointerTrail();
    t.push(at(0, "a", 0.9), 0);
    t.push(at(2, "b", 0.1), 40);
    expect(t.at(20 + INTERP_DELAY_MS)).toEqual(at(0, "a", 0.9));
    expect(t.at(40 + INTERP_DELAY_MS)).toEqual(at(2, "b", 0.1));
  });

  it("glides in from a resting position after a pause", () => {
    const t = new PointerTrail();
    t.push(at(0), 0);
    t.push(at(8), 5000);
    // Halfway through one interval after the new sample's arrival.
    const mid = t.at(5000 + INTERP_DELAY_MS / 2)!;
    expect(mid.beats).toBeCloseTo(4);
    expect(t.at(5000 + INTERP_DELAY_MS)).toEqual(at(8));
  });

  it("glides a piano-roll pointer within a clip and snaps between clips", () => {
    const ed = (clip: string, beats: number, pitch: number) => ({ beats: 0, track: null, y: 0, editor: { clip, beats, pitch } });
    const t = new PointerTrail();
    t.push(ed("c1", 0, 60), 0);
    t.push(ed("c1", 4, 64), 40);
    expect(t.at(20 + INTERP_DELAY_MS)).toEqual(ed("c1", 2, 62));
    t.push(ed("c2", 1, 50), 80);
    expect(t.at(60 + INTERP_DELAY_MS)).toEqual(ed("c1", 4, 64));
    // Out of the piano roll into the arranger: snaps too.
    t.push(at(3), 120);
    expect(t.at(100 + INTERP_DELAY_MS)).toEqual(ed("c2", 1, 50));
  });
});
