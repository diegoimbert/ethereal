import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import type { EqShape } from "@/generated";
import { bandCurves, magnitudeDb, summedDb } from "./eqResponse";

interface Vector {
  shape: EqShape;
  freq: number;
  gain_db: number;
  q: number;
  at_hz: number;
  sample_rate: number;
  db: number;
}

const PATH = resolve(import.meta.dirname, "../../../../../../crates/ether-protocol/tests/fixtures/eq_response_vectors.json");
const vectors = JSON.parse(readFileSync(PATH, "utf8")) as Vector[];

describe("EQ response (parity with ether_protocol::eq_response)", () => {
  it("matches every Rust vector within 1e-6 dB", () => {
    expect(vectors.length).toBeGreaterThan(300);
    for (const v of vectors) {
      const got = magnitudeDb(v.shape, v.freq, v.gain_db, v.q, v.at_hz, v.sample_rate);
      expect(Math.abs(got - v.db), JSON.stringify(v)).toBeLessThanOrEqual(1e-6);
    }
  });

  it("covers every shape", () => {
    const shapes = new Set(vectors.map((v) => v.shape));
    expect([...shapes].sort()).toEqual(["BandPass", "Bell", "HighCut", "HighCut24", "HighShelf", "LowCut", "LowCut24", "LowShelf", "Notch"]);
  });

  it("sums enabled bands in dB and ignores disabled ones", () => {
    const freqs = [100, 1000, 10000];
    const bell = { shape: "Bell" as const, freq: 1000, gainDb: 6, q: 1, on: true };
    const curves = bandCurves([bell, bell, { ...bell, on: false }], freqs, 48000);
    const sum = summedDb(curves, freqs.length);
    expect(sum[1]).toBeCloseTo(12, 9);
    expect(curves[2]).toEqual([0, 0, 0]);
  });
});
