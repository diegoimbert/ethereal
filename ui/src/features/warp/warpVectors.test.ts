/**
 * Runs the shared warp mapping vectors (`crates/ether-core/src/warp/vectors.json`, also
 * run by ether-core and ether-controller), so the UI's waveform mapping stays in lockstep
 * with what the engine plays.
 */

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import type { AudioContent, WarpSettings } from "@/generated";
import { clipSourceMapper, compileWarp } from "./warpMap";

interface Vector {
  name: string;
  warp: WarpSettings;
  markers: Array<[number, number]>;
  ref_bpm: number;
  transpose: number;
  anchor: number;
  stretch: boolean;
  compiled: Array<[number, number]> | null;
  points: Array<[number, number]>;
}

const PATH = resolve(import.meta.dirname, "../../../../crates/ether-core/src/warp/vectors.json");
const { vectors } = JSON.parse(readFileSync(PATH, "utf8")) as { vectors: Vector[] };

describe("shared warp vectors", () => {
  it("has vectors", () => expect(vectors.length).toBeGreaterThan(5));
  for (const v of vectors) {
    it(v.name, () => {
      const markers = v.markers.map(([beat, source]) => ({ beat, source }));
      expect(compileWarp(v.warp, markers)).toEqual(v.compiled);
      const content: AudioContent = { media: "M", gain: 0, transpose: v.transpose, fade_in: 0, fade_out: 0, fade_in_curve: { type: "Linear" }, fade_out_curve: { type: "Linear" }, reversed: false, warp: v.warp };
      const map = clipSourceMapper(content, markers, v.ref_bpm, v.anchor, v.stretch ? "tauri" : "wasm");
      for (const [c, s] of v.points) expect(Math.abs(map(c) - s), `c=${c}`).toBeLessThan(1e-9);
    });
  }
});
