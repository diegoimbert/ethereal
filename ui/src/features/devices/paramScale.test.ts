import { describe, expect, it } from "vitest";
import type { ParamInfo, ParamScale } from "@/generated";
import {
  formatDb,
  formatPan,
  formatParam,
  labelIndex,
  labelValue,
  paramToNormalized,
  paramToPlain,
  scaleToNormalized,
  scaleToPlain,
} from "./paramScale";

const close = (a: number, b: number) => Math.abs(a - b) < 1e-9;

function info(p: Partial<ParamInfo>): ParamInfo {
  return {
    id: 0,
    name: "P",
    group: null,
    unit: "None",
    min: 0,
    max: 1,
    default: 0,
    scale: { type: "Linear" },
    labels: null,
    automatable: true,
    hidden: false,
    ...p,
  };
}

describe("scale mapping (mirrors ether_protocol::devices test vectors)", () => {
  it("matches the Rust vectors", () => {
    expect(close(scaleToPlain({ type: "Linear" }, -1, 1, 0.5), 0)).toBe(true);
    expect(close(scaleToPlain({ type: "Log" }, 20, 20000, 0.5), 632.4555320336759)).toBe(true);
    expect(close(scaleToPlain({ type: "Power", exponent: 2 }, 0, 100, 0.5), 25)).toBe(true);
    expect(close(scaleToPlain({ type: "Fader" }, -70, 6, 1), 6)).toBe(true);
    expect(close(scaleToPlain({ type: "Fader" }, -70, 6, 0), -70)).toBe(true);
  });

  it("round-trips", () => {
    const scales: ParamScale[] = [{ type: "Linear" }, { type: "Log" }, { type: "Power", exponent: 3 }, { type: "Fader" }];
    for (const scale of scales) {
      const [min, max] = scale.type === "Fader" ? [-70, 6] : [20, 20000];
      for (let i = 1; i < 10; i++) {
        const n = i / 10;
        const back = scaleToNormalized(scale, min, max, scaleToPlain(scale, min, max, n));
        expect(Math.abs(back - n)).toBeLessThan(1e-9);
      }
    }
  });

  it("clamps out-of-range input", () => {
    expect(scaleToPlain({ type: "Linear" }, 0, 10, 2)).toBe(10);
    expect(scaleToNormalized({ type: "Linear" }, 0, 10, -5)).toBe(0);
    expect(scaleToNormalized({ type: "Linear" }, 3, 3, 3)).toBe(0);
  });
});

describe("ParamInfo helpers", () => {
  const wave = info({ min: 0, max: 3, labels: ["Sine", "Saw", "Square", "Tri"] });

  it("snaps enum params to steps", () => {
    expect(paramToPlain(wave, 0.4)).toBe(1);
    expect(paramToPlain(wave, 0.9)).toBe(3);
    expect(labelIndex(wave, 2)).toBe(2);
    expect(labelValue(wave, 1)).toBe(1);
    expect(formatParam(wave, 1)).toBe("Saw");
  });

  it("maps log params through the scale", () => {
    const cutoff = info({ min: 20, max: 20000, scale: { type: "Log" }, unit: "Hertz" });
    expect(close(paramToNormalized(cutoff, 632.4555320336759), 0.5)).toBe(true);
    expect(formatParam(cutoff, 8000)).toBe("8 kHz");
    expect(formatParam(cutoff, 440)).toBe("440 Hz");
  });

  it("formats units", () => {
    expect(formatParam(info({ unit: "Milliseconds", max: 5000 }), 5)).toBe("5 ms");
    expect(formatParam(info({ unit: "Percent", max: 100 }), 35)).toBe("35 %");
    expect(formatParam(info({ unit: "Semitones", min: -12, max: 12 }), 3)).toBe("+3 st");
    expect(formatDb(-144)).toBe("-inf dB");
    expect(formatDb(-6)).toBe("-6.0 dB");
    expect(formatPan(0)).toBe("C");
    expect(formatPan(-0.25)).toBe("25L");
    expect(formatPan(1)).toBe("100R");
  });
});
