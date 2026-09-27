import { describe, expect, it } from "vitest";
import { CHROMATIC_SCALE, getPitchClass, getScaleNotes, isNoteInScale, resolveScale, SCALE_KINDS } from "./scales";
import { createPitchRows, pitchToY, rowPitchDelta, yToPitch } from "@/features/piano-roll/geometry";

describe("musical scales", () => {
  it("wraps pitch classes and transposes scales through B", () => {
    expect([0, 60, 127, -1, -12].map(getPitchClass)).toEqual([0, 0, 7, 11, 0]);
    expect(getScaleNotes(0, "Minor")).toEqual([0, 2, 3, 5, 7, 8, 10]);
    expect(getScaleNotes(11, "Major")).toEqual([11, 1, 3, 4, 6, 8, 10]);
    expect(getScaleNotes(0, "HarmonicMinor")).toEqual([0, 2, 3, 5, 7, 8, 11]);
    expect(getScaleNotes(0, "MelodicMinor")).toEqual([0, 2, 3, 5, 7, 9, 11]);
    expect(getScaleNotes(0, "Blues")).toEqual([0, 3, 5, 6, 7, 10]);
  });
  it("uses the same membership in every octave and root", () => {
    for (const kind of SCALE_KINDS) for (let root = 0; root < 12; root++) {
      const pitches = getScaleNotes(root, kind);
      expect(new Set(pitches).size).toBe(pitches.length);
      expect(pitches[0]).toBe(root);
      for (let midi = 0; midi < 128; midi++) {
        expect(isNoteInScale(midi, root, kind)).toBe(pitches.includes(midi % 12));
        expect(isNoteInScale(midi, root, "Chromatic")).toBe(true);
      }
    }
  });
  it("resolves project inheritance, custom overrides and unrestricted tracks", () => {
    const project = { root: 2, kind: "Dorian" } as const;
    const custom = { root: 9, kind: "Minor" } as const;
    expect(resolveScale(project, { type: "FollowProject" })).toEqual(project);
    expect(resolveScale(project, { type: "Custom", scale: custom })).toEqual(custom);
    expect(resolveScale(project, { type: "Chromatic" })).toEqual(CHROMATIC_SCALE);
  });
  it("packs visible rows continuously with reversible pointer coordinates at MIDI boundaries", () => {
    for (const kind of SCALE_KINDS) for (let root = 0; root < 12; root++) {
      const rows = createPitchRows({ root, kind }, true);
      rows.forEach((pitch, index) => {
        expect(pitchToY(pitch, 12, rows)).toBe(index * 12);
        expect(yToPitch(index * 12 + 6, 12, rows)).toBe(pitch);
      });
      expect(yToPitch(-100, 12, rows)).toBe(rows[0]);
      expect(yToPitch(10000, 12, rows)).toBe(rows.at(-1));
    }
    expect(createPitchRows({ root: 0, kind: "Minor" }, false)).toHaveLength(128);
  });
  it("counts vertical drags in visible rows (folded or not), clamped at the ends", () => {
    const all = createPitchRows();
    expect(rowPitchDelta(60, -12, 12, all)).toBe(1);
    expect(rowPitchDelta(60, 30, 12, all)).toBe(-3);
    expect(rowPitchDelta(126, -60, 12, all)).toBe(1);
    const cMinor = createPitchRows({ root: 0, kind: "Minor" }, true);
    // C -> D -> D# going up; C -> A# -> G# going down.
    expect(rowPitchDelta(60, -12, 12, cMinor)).toBe(2);
    expect(rowPitchDelta(60, -24, 12, cMinor)).toBe(3);
    expect(rowPitchDelta(60, 24, 12, cMinor)).toBe(-4);
    expect(60 + rowPitchDelta(60, -100000, 12, cMinor)).toBe(cMinor[0]);
    expect(60 + rowPitchDelta(60, 100000, 12, cMinor)).toBe(cMinor.at(-1));
  });
});
