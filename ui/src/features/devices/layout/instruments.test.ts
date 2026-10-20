// The built-in instruments' declared layouts as the renderer
// resolves them (mock descriptors = the Rust ones, `v02_descriptors.rs` keeps them equal),
// their window widths, and the cents unit.
import { describe, expect, it } from "vitest";
import type { BuiltinDeviceType, Device, ParamInfo } from "@/generated";
import { BUILTIN_DESCRIPTORS } from "@/transport";
import { formatParam } from "@/domain/paramScale";
import { deviceWindowWidth } from "../window/sizes";
import { filterMagnitudeDb, filterShapeOf } from "./curves";
import { boundParams, resolveLayout } from "./model";
import { formatValue } from "./values";

const INSTRUMENTS: BuiltinDeviceType[] = ["Synth", "PolySynth", "Sampler", "MultiSampler", "DrumRack", "FmSynth"];

/** Section id → title and the names of the params it binds (the panel's outline). */
function outline(type: BuiltinDeviceType): string[] {
  const d = BUILTIN_DESCRIPTORS[type];
  const name = new Map(d.params.map((p) => [p.id, p.name]));
  return d.layout!.sections.map(
    (s) => `${s.id} (${s.title ?? "-"}): ${s.items.flatMap((i) => boundParams(i.widget).map((p) => name.get(p) ?? `#${p}`)).join(", ")}`,
  );
}

describe("instrument layouts", () => {
  it("show every visible param (nothing folded under More), never a hidden one", () => {
    for (const type of INSTRUMENTS) {
      const d = BUILTIN_DESCRIPTORS[type];
      const r = resolveLayout(d);
      expect(r.declared, type).toBe(true);
      expect(r.more, `${type}: folded params`).toBeNull();
      const hidden = new Set(d.params.filter((p) => p.hidden).map((p) => p.id));
      for (const s of r.main.sections)
        for (const i of s.items) for (const p of boundParams(i.widget)) expect(hidden.has(p), `${type} shows hidden #${p}`).toBe(false);
    }
  });

  it("poly synth: a unison section (voices, detune, spread, blend) and a voice section", () => {
    expect(outline("PolySynth")).toMatchInlineSnapshot(`
      [
        "osc1 (Osc 1): Type, Position, Table, Octave, Semitones, Fine, Pulse Width, Level",
        "osc2 (Osc 2): Type, Position, Table, Octave, Semitones, Fine, Pulse Width, Level",
        "sub-noise (Sub / Noise): Sub Level, Sub Octave, Noise Level, Noise Color",
        "filter (Filter): Cutoff, Resonance, Type, Drive, Key Tracking, Env Amount",
        "filter-env (Filter Envelope): Attack, Decay, Sustain, Release",
        "amp-env (Amp Envelope): Attack, Decay, Sustain, Release",
        "mod-env (Mod Envelope): Attack, Decay, Sustain, Release, Target, Amount",
        "lfo1 (LFO 1): Shape, Rate, Amount, Sync, Sync Rate, Target",
        "lfo2 (LFO 2): Shape, Rate, Amount, Sync, Sync Rate, Target",
        "unison (Unison): Unison Voices, Unison Detune, Spread, Unison Blend",
        "voice (Pitch / Voice): Transpose, Fine, Mode, Voices, Bend Range, Glide, Drift",
        "output (Output): Volume, Velocity",
      ]
    `);
    const legacy = BUILTIN_DESCRIPTORS.PolySynth.params.filter((p) => p.hidden).map((p) => p.name);
    expect(legacy).toEqual(["Unison (legacy)", "Detune (legacy)"]);
  });

  it("fm synth: the algorithm picker, four operator columns, then pitch / LFO / filter / output", () => {
    expect(outline("FmSynth")).toMatchInlineSnapshot(`
      [
        "op-a (Operator A): On, Wave, Coarse, Fine, Fixed, Frequency, Level, Velocity, Feedback, Attack, Decay, Sustain, Release",
        "op-b (Operator B): On, Wave, Coarse, Fine, Fixed, Frequency, Level, Velocity, Feedback, Attack, Decay, Sustain, Release",
        "op-c (Operator C): On, Wave, Coarse, Fine, Fixed, Frequency, Level, Velocity, Feedback, Attack, Decay, Sustain, Release",
        "op-d (Operator D): On, Wave, Coarse, Fine, Fixed, Frequency, Level, Velocity, Feedback, Attack, Decay, Sustain, Release",
        "algorithm (Algorithm): Algorithm",
        "pitch-voice (Pitch / Voice): Transpose, Fine, Mode, Voices, Bend Range, Glide",
        "lfo (LFO): Shape, Rate, To Pitch, To Filter, To Volume",
        "filter (Filter): Cutoff, Resonance, Type, On, Env Amount, Key Tracking",
        "filter-env (Filter Envelope): Attack, Decay, Sustain, Release",
        "output (Output): Volume, Tone, Antialias",
      ]
    `);
    const ops = BUILTIN_DESCRIPTORS.FmSynth.layout!.sections.filter((s) => s.id.startsWith("op-"));
    expect(ops.map((s) => s.title)).toEqual(["Operator A", "Operator B", "Operator C", "Operator D"]);
  });

  it("open wider for the big instruments", () => {
    const of = (type: BuiltinDeviceType) => deviceWindowWidth({ kind: { type: "Builtin", device: { type } } } as unknown as Pick<Device, "kind">);
    expect(of("PolySynth")).toBeGreaterThan(of("Synth"));
    expect(of("MultiSampler")).toBeGreaterThanOrEqual(of("Sampler"));
    expect(of("Sampler")).toBeGreaterThan(of("Synth"));
    expect(of("DrumRack")).toBeGreaterThan(of("Synth"));
    expect(of("FmSynth")).toBeGreaterThan(of("PolySynth"));
    // The EQ's eight bands get ~760 px.
    expect(of("Eq")).toBeGreaterThanOrEqual(740);
    expect(of("Eq")).toBeLessThanOrEqual(780);
  });
});

describe("cents", () => {
  const detune = BUILTIN_DESCRIPTORS.PolySynth.params.find((p) => p.name === "Unison Detune") as ParamInfo;
  it("is the unison detune's unit", () => {
    expect(detune.unit).toBe("Cents");
    expect([detune.min, detune.max, detune.default]).toEqual([0, 100, 20]);
  });
  it("formats as ct (one decimal under 10, signed when bipolar)", () => {
    expect(formatParam(detune, 20)).toBe("20 ct");
    expect(formatParam(detune, 4.24)).toBe("4.2 ct");
    expect(formatValue(detune, 0)).toBe("0 ct");
    expect(formatParam({ ...detune, min: -100 }, 12)).toBe("+12 ct");
  });
});

describe("filter curve", () => {
  it("draws a switched-off filter flat", () => {
    const off = filterShapeOf("Off");
    for (const f of [20, 1000, 20000]) expect(Math.abs(filterMagnitudeDb(off, f, 500, 4))).toBeLessThan(1e-9);
    expect(filterShapeOf("Low-pass").kind).toBe("lowpass");
  });
});
