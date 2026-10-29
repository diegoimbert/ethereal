import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import type { GraphicEqShape } from "@/generated";
import { SHAPES, bandCurve, graphicEqDb, graphicEqStages } from "./response";

interface Vector {
  shape: GraphicEqShape;
  freq: number;
  gain_db: number;
  q: number;
  slope: number;
  at_hz: number;
  sample_rate: number;
  db: number;
}

const PATH = resolve(import.meta.dirname, "../../../../../../crates/ether-protocol/tests/fixtures/graphic_eq_vectors.json");
const vectors = JSON.parse(readFileSync(PATH, "utf8")) as Vector[];

describe("graphic EQ response (parity with ether_protocol::eq_response)", () => {
  it("matches every Rust vector within 1e-6 dB", () => {
    expect(vectors.length).toBeGreaterThan(500);
    for (const v of vectors) {
      const got = graphicEqDb(v.shape, v.freq, v.gain_db, v.q, v.slope, v.at_hz, v.sample_rate);
      expect(Math.abs(got - v.db), JSON.stringify(v)).toBeLessThanOrEqual(1e-6);
    }
  });

  it("covers every shape, in the device's Type order", () => {
    expect(new Set(vectors.map((v) => v.shape))).toEqual(new Set(SHAPES));
  });

  it("a bell at +6 dB gives +6 dB at its frequency; flat shapes have no stage", () => {
    expect(graphicEqDb("Bell", 1000, 6, 1, 1, 1000, 48000)).toBeCloseTo(6, 9);
    for (const s of ["Bell", "LowShelf", "HighShelf", "TiltShelf", "FlatTilt"] as const) expect(graphicEqStages(s, 1000, 0, 1, 1, 48000)).toEqual([]);
    expect(bandCurve({ shape: "Bell", freq: 1000, gainDb: 6, q: 1, slope: 1, on: false }, [100, 1000], 48000)).toEqual([0, 0]);
  });
});
