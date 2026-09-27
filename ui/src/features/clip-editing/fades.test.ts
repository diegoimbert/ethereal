import { describe, expect, it } from "vitest";
import type { FadeCurve } from "@/generated";
import { fadeGain } from "./fades";
import vectors from "./fadeVectors.json";

describe("fadeGain (mirror of ether_core::fades::fade_gain)", () => {
  it("has the Rust endpoints and shapes", () => {
    for (const c of [{ type: "Linear" }, { type: "EqualPower" }, { type: "Curve", tension: 0.7 }, { type: "Curve", tension: -0.7 }] as const) {
      expect(fadeGain(c, 0)).toBe(0);
      expect(fadeGain(c, 1)).toBeCloseTo(1, 6);
    }
    expect(fadeGain({ type: "Linear" }, 0.25)).toBe(0.25);
    const a = fadeGain({ type: "EqualPower" }, 0.3);
    const b = fadeGain({ type: "EqualPower" }, 0.7);
    expect(a * a + b * b).toBeCloseTo(1, 6);
    expect(fadeGain({ type: "Curve", tension: 0.5 }, 0.5)).toBeCloseTo(0.25, 9);
    expect(fadeGain({ type: "Curve", tension: -0.5 }, 0.5)).toBeGreaterThan(0.5);
  });

  it("matches the shared vectors (crates/ether-core/tests/fades_vectors.rs runs the same file)", () => {
    expect(vectors.cases.length).toBeGreaterThan(2);
    for (const c of vectors.cases) {
      for (const [x, want] of c.points as [number, number][]) {
        expect(Math.abs(fadeGain(c.curve as FadeCurve, x) - want)).toBeLessThanOrEqual(vectors.tolerance);
      }
    }
  });
});
