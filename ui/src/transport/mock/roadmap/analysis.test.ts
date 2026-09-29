/** MockTransport: `Analysis` watches and simulated frames (v0.2, fx-analysis). */
import { describe, expect, it } from "vitest";
import type { Event, Project } from "@/generated";
import { MockAnalysis, MOCK_SPECTRUM_BINS } from "./analysis";

function host(types: Record<string, string>, params: Record<string, Record<number, number>> = {}) {
  const events: Event[] = [];
  const devices = Object.fromEntries(Object.entries(types).map(([id, type]) => [id, { id, kind: { type: "Builtin", device: { type } }, params: params[id] ?? {} }]));
  return { events, host: { project: () => ({ devices }) as unknown as Project, emit: (e: Event) => void events.push(e) } };
}

describe("MockAnalysis", () => {
  it("refcounts watched devices", () => {
    const a = new MockAnalysis();
    expect(a.command({ type: "Watch", device: "d1" })).toEqual({ type: "Unit" });
    a.command({ type: "Watch", device: "d1" });
    a.command({ type: "Unwatch", device: "d1" });
    expect(a.watched.has("d1")).toBe(true);
    a.command({ type: "Unwatch", device: "d1" });
    expect(a.watched.size).toBe(0);
  });

  it("emits spectrum and tuner frames for watched devices only", () => {
    const { events, host: h } = host({ s: "SpectrumAnalyzer", t: "Tuner", e: "Eq", c: "Compressor" }, { t: { 0: 440 } });
    const a = new MockAnalysis(h);
    a.step();
    expect(events).toEqual([]);
    for (const d of ["s", "t", "e", "c"]) a.command({ type: "Watch", device: d });
    a.step();
    const frames = events.map((e) => (e.type === "Analysis" ? e.event : null));
    const of = (d: string) => frames.filter((f) => f?.device === d).map((f) => f!.data);
    const [spectrum] = of("s");
    expect(spectrum).toMatchObject({ type: "Spectrum", min_hz: 20, max_hz: 20000, stage: "Post" });
    expect(spectrum!.type === "Spectrum" && spectrum!.bins_db.length).toBe(MOCK_SPECTRUM_BINS);
    expect(of("e").map((f) => f.type === "Spectrum" && f.stage)).toEqual(["Pre", "Post"]);
    const [tuner] = of("t");
    expect(tuner).toMatchObject({ type: "Tuner", note: 45 });
    expect(of("c")).toEqual([]);
    a.command({ type: "Unwatch", device: "s" });
    events.length = 0;
    a.step();
    expect(events.some((e) => e.type === "Analysis" && e.event.device === "s")).toBe(false);
  });

  it("tuner cents follow the reference param", () => {
    const at = (reference: number) => {
      const { events, host: h } = host({ t: "Tuner" }, { t: { 0: reference } });
      const a = new MockAnalysis(h);
      a.command({ type: "Watch", device: "t" });
      a.step();
      const e = events[0]!;
      return e.type === "Analysis" && e.event.data.type === "Tuner" ? e.event.data.cents : NaN;
    };
    // 1200·log2(440/442) ≈ -7.85 ct lower against a higher reference.
    expect(at(440) - at(442)).toBeCloseTo(7.85, 0);
  });
});
