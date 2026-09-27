import { describe, expect, it } from "vitest";
import type { AudioContent, WarpSettings } from "@/generated";
import { beatAtSource, clipSourceMapper, compileWarp, complexStretches, sourceSecondsAt, type Pin } from "./warpMap";

const on = (mode: "Repitch" | "Complex", source_bpm: number | null = null): WarpSettings => ({ enabled: true, mode, source_bpm });
const m = (beat: number, source: number) => ({ beat, source });

describe("compileWarp (mirrors ether-controller warp::warp_desc)", () => {
  it("is null when unwarped or when the tempo is unknown", () => {
    expect(compileWarp({ ...on("Complex", 120), enabled: false }, [m(0, 0), m(4, 2)])).toBeNull();
    expect(compileWarp(on("Complex"), [])).toBeNull();
    expect(compileWarp(on("Complex"), [m(1, 1)])).toBeNull();
  });

  it("derives the slope from source_bpm with fewer than two markers", () => {
    expect(compileWarp(on("Repitch", 100), [])).toEqual([
      [0, 0],
      [1, 0.6],
    ]);
    expect(compileWarp(on("Complex", 120), [m(2, 1.5)])).toEqual([
      [2, 1.5],
      [3, 2],
    ]);
  });

  it("sorts and dedups markers (first within epsilon wins)", () => {
    expect(compileWarp(on("Complex", 120), [m(8, 3), m(0, 0), m(4, 2.5), m(4 + 1e-9, 9)])).toEqual([
      [0, 0],
      [4, 2.5],
      [8, 3],
    ]);
  });
});

describe("sourceSecondsAt (mirrors ether-core sched::source_seconds)", () => {
  // Same vectors as `sched::tests::warp_mapping`.
  const pins: Pin[] = [
    [0, 0],
    [4, 1],
    [8, 3],
  ];
  it("is piecewise linear with edge-slope extrapolation", () => {
    expect(sourceSecondsAt(pins, 120, 2)).toBe(0.5);
    expect(sourceSecondsAt(pins, 120, 6)).toBe(2);
    expect(sourceSecondsAt(pins, 120, 10)).toBe(4);
    expect(sourceSecondsAt(pins, 120, -4)).toBe(-1);
  });
  it("plays unwarped clips at the song tempo at the clip start", () => {
    expect(sourceSecondsAt(null, 120, 4)).toBe(2);
    expect(sourceSecondsAt(null, 60, 4)).toBe(4);
  });
});

describe("clipSourceMapper (mirrors ether-core warp::repitch_source_seconds)", () => {
  const content = (warp: WarpSettings, transpose: number): AudioContent => ({
    media: "M",
    gain: 0,
    transpose,
    fade_in: 0,
    fade_out: 0,
    fade_in_curve: { type: "Linear" },
    fade_out_curve: { type: "Linear" },
    reversed: false,
    warp,
  });
  const markers = [m(0, 0), m(4, 2)];

  it("repitch transpose speeds playback around the clip offset", () => {
    const f = clipSourceMapper(content(on("Repitch"), 12), markers, 120, 2, "tauri");
    expect(f(2)).toBeCloseTo(1); // anchor unchanged
    expect(f(4)).toBeCloseTo(3); // 1 + (2 - 1) * 2
    const unwarped = clipSourceMapper(content({ ...on("Complex"), enabled: false }, -12), [], 120, 0, "tauri");
    expect(unwarped(4)).toBeCloseTo(1);
  });

  it("complex transpose keeps the mapping, except on the web where it repitches", () => {
    const native = clipSourceMapper(content(on("Complex"), 12), markers, 120, 0, "tauri");
    expect(native(4)).toBeCloseTo(2);
    const web = clipSourceMapper(content(on("Complex"), 12), markers, 120, 0, "wasm");
    expect(web(4)).toBeCloseTo(4);
    expect(complexStretches("wasm")).toBe(false);
    expect(complexStretches("mock")).toBe(true);
  });

  it("inverts by bisection", () => {
    const f = clipSourceMapper(content(on("Complex"), 0), [m(0, 0), m(4, 1), m(8, 3)], 120, 0, "tauri");
    expect(beatAtSource(f, 2)).toBeCloseTo(6, 6);
    expect(beatAtSource(f, 0.5)).toBeCloseTo(2, 6);
  });
});
