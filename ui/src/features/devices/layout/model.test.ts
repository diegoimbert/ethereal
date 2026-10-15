import { describe, expect, it } from "vitest";
import type { DeviceLayout, ParamInfo, Widget } from "@/generated";
import { BUILTIN_DESCRIPTORS } from "@/transport";
import { boundParams, genericLayout, genericWidget, isGenericSection, referencedParams, resolveLayout } from "./model";
import { filterMagnitudeDb, filterShapeOf, freqToX, transfer, waveAt, waveOf, xToFreq } from "./curves";
import { formatValue, snapPlain, stepDecimals, toPlain } from "./values";

function p(id: number, name: string, group: string | null, extra: Partial<ParamInfo> = {}): ParamInfo {
  return { id, name, group, unit: "None", min: 0, max: 1, default: 0, scale: { type: "Linear" }, labels: null, automatable: true, hidden: false, ...extra };
}

describe("layout model", () => {
  it("binds the params every widget references (mirrors layouts.rs)", () => {
    const cases: Array<[Widget, number[]]> = [
      [{ type: "Knob", param: 3 }, [3]],
      [{ type: "Envelope", attack: 1, decay: 2, sustain: 3, release: 4, delay: null, hold: 9 }, [1, 2, 3, 4, 9]],
      [{ type: "FilterCurve", cutoff: 5, resonance: 6, mode: 7, drive: null, gain: null }, [5, 6, 7]],
      [{ type: "StepEditor", first: 10, count: 3 }, [10, 11, 12]],
      [{ type: "Crossover", frequencies: [2, 4] }, [2, 4]],
      [{ type: "SampleWaveform", start: 6, end: null }, [6]],
      [{ type: "Meter", index: 0, min_db: -24, max_db: 0 }, []],
      [
        {
          type: "EqCurve",
          bands: [{ on: 0, kind: 1, shapes: ["Bell"], freq: 2, gain: 3, q: null }],
          crossovers: [9],
          spectrum: "Post",
        },
        [0, 1, 2, 3, 9],
      ],
    ];
    for (const [w, ids] of cases) expect(boundParams(w)).toEqual(ids);
  });

  it("the generic layout keeps the owner's look: leading groups large, the rest under More", () => {
    const params = [
      p(0, "A", "One"),
      p(1, "B", "One"),
      p(2, "C", "Two"),
      p(3, "D", "Two"),
      p(4, "E", "Three"),
      p(5, "F", "Three"),
      p(6, "G", "Three", { hidden: true }),
      p(7, "H", "Four"),
    ];
    const r = genericLayout(params);
    expect(r.declared).toBe(false);
    expect(r.main.sections.map((s) => s.title)).toEqual(["One", "Two"]);
    expect(r.main.sections.every(isGenericSection)).toBe(true);
    expect(r.main.sections[0]!.items.every((i) => i.size === "Large")).toBe(true);
    expect(r.more!.sections.map((s) => s.title)).toEqual(["Three", "Four"]);
    expect(r.more!.sections[0]!.items.every((i) => i.size === "Medium")).toBe(true);
    expect(r.moreCount).toBe(3);
    // Small devices show everything.
    expect(genericLayout(params.slice(0, 4)).more).toBeNull();
  });

  it("chooses toggle / choice / knob for generic params", () => {
    expect(genericWidget(p(0, "On", null, { labels: ["Off", "On"] })).type).toBe("Toggle");
    expect(genericWidget(p(0, "On", null, { unit: "Toggle" })).type).toBe("Toggle");
    expect(genericWidget(p(0, "Mode", null, { labels: ["A", "B", "C"], max: 2 })).type).toBe("Choice");
    expect(genericWidget(p(0, "Gain", null)).type).toBe("Knob");
  });

  it("a declared layout shows its sections; unreferenced visible params fold under More", () => {
    const params = [p(0, "A", "G"), p(1, "B", "G"), p(2, "C", "H"), p(3, "D", "H", { hidden: true })];
    const layout: DeviceLayout = {
      sections: [{ id: "main", title: "Main", span: 2, columns: 2, items: [{ widget: { type: "Knob", param: 0 }, size: "Large", colspan: 1, label: null }] }],
    };
    const r = resolveLayout({ params, layout });
    expect(r.declared).toBe(true);
    expect(r.main).toBe(layout);
    expect(r.moreCount).toBe(2);
    expect(r.more!.sections.flatMap((s) => s.items.map((i) => boundParams(i.widget)[0]))).toEqual([1, 2]);
    expect(resolveLayout({ params: params.slice(0, 1), layout }).more).toBeNull();
    // An empty layout falls back to the generic one.
    expect(resolveLayout({ params, layout: { sections: [] } }).declared).toBe(false);
  });

  it("every built-in mock descriptor with a layout references existing params only", () => {
    for (const d of Object.values(BUILTIN_DESCRIPTORS)) {
      if (!d.layout) continue;
      const ids = new Set(d.params.map((x) => x.id));
      for (const id of referencedParams(d.layout)) {
        if (d.layout.sections.some((s) => s.items.some((i) => i.widget.type === "Macros"))) continue;
        expect(ids.has(id), `${d.name}: param ${id}`).toBe(true);
      }
    }
  });
});

describe("values", () => {
  const semis = p(0, "Transpose", null, { unit: "Semitones", min: -24, max: 24, step: 1 });
  it("snaps to ParamInfo.step like Rust ParamInfo::snap", () => {
    expect(snapPlain(semis, 6.6)).toBe(7);
    expect(snapPlain(semis, -30)).toBe(-24);
    expect(snapPlain(p(0, "x", null, { min: 0, max: 1, step: 0.1 }), 0.33)).toBe(0.3);
    expect(snapPlain(p(0, "x", null, { min: 0, max: 1 }), 0.33)).toBe(0.33);
    expect(toPlain(semis, 0.5 + 0.3 / 48)).toBe(0);
    expect(stepDecimals(0.25)).toBe(2);
    expect(stepDecimals(1)).toBe(0);
  });

  it("formats with step, unit and labels", () => {
    expect(formatValue(semis, 7.2)).toBe("+7 st");
    expect(formatValue(p(0, "Voices", null, { min: 1, max: 16, step: 1 }), 3.4)).toBe("3");
    expect(formatValue(p(0, "Wave", null, { labels: ["Sine", "Saw"], max: 1, step: 1 }), 1)).toBe("Saw");
    expect(formatValue(p(0, "Cutoff", null, { unit: "Hertz", min: 20, max: 20000 }), 1200)).toBe("1.2 kHz");
    expect(formatValue(p(0, "Mix", null, { unit: "Percent", min: 0, max: 100, step: 1 }), 33.3)).toBe("33 %");
  });
});

describe("curves", () => {
  it("log frequency axis round-trips", () => {
    expect(freqToX(20)).toBe(0);
    expect(freqToX(20000)).toBeCloseTo(1, 9);
    expect(xToFreq(freqToX(1000))).toBeCloseTo(1000, 6);
  });

  it("filter shapes: -3 dB at cutoff for a Butterworth low-pass, steep = doubled", () => {
    const lp = filterShapeOf("Low-pass 12");
    expect(lp).toEqual({ kind: "lowpass", steep: false });
    expect(filterMagnitudeDb(lp, 1000, 1000, Math.SQRT1_2)).toBeCloseTo(-3.0103, 3);
    expect(filterMagnitudeDb(lp, 10, 1000, Math.SQRT1_2)).toBeCloseTo(0, 3);
    expect(filterMagnitudeDb({ kind: "lowpass", steep: true }, 1000, 1000, Math.SQRT1_2)).toBeCloseTo(-6.0206, 3);
    expect(filterShapeOf("High-pass 24")).toEqual({ kind: "highpass", steep: true });
    expect(filterShapeOf("Notch").kind).toBe("notch");
    expect(filterShapeOf("Band-pass").kind).toBe("bandpass");
    expect(filterMagnitudeDb({ kind: "peak", steep: false }, 1000, 1000, 1, 6)).toBeCloseTo(6, 6);
  });

  it("transfer curves pass through the origin and stay in -1..1", () => {
    for (const drive of [1, 5, 20]) {
      expect(transfer(0, drive, 0.3, 0.2)).toBeCloseTo(0, 9);
      for (let x = -1; x <= 1; x += 0.25) expect(Math.abs(transfer(x, drive, 0.5, 0.1))).toBeLessThanOrEqual(1);
    }
  });

  it("wave shapes from labels", () => {
    expect(waveOf("Saw")).toBe("saw");
    expect(waveOf("Square")).toBe("square");
    expect(waveOf("S&H")).toBe("random");
    expect(waveOf("Wavetable")).toBe("table");
    expect(waveAt("sine", 0.25)).toBeCloseTo(1, 9);
    expect(waveAt("triangle", 0.25)).toBeCloseTo(1, 9);
    expect(waveAt("table", 0.25, 0)).toBeCloseTo(1, 9);
  });
});
