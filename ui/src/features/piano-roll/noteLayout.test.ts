import { describe, expect, it } from "vitest";
import type { Note } from "@/generated";
import type { Rgba } from "@/features/arrangement/gl/palette";
import { CHROMATIC_SCALE } from "@/domain/scales";
import { createPitchRows, pitchRow } from "./geometry";
import { InstanceList } from "./noteCanvas";
import {
  buildNotesLayer,
  buildRowsLayer,
  gridCursor,
  gridPalette,
  grayscale,
  hitNote,
  noteBox,
  rowTable,
  tagged,
  type GridColors,
  type GridGeometry,
  type NotesLayerInput,
  type SceneTag,
} from "./noteLayout";

const KEY_H = 10;
const vp = { pxPerBeat: 40, scrollBeats: 0 };
const geo: GridGeometry = { vp, keyH: KEY_H, rows: createPitchRows() };
const y = (pitch: number) => (127 - pitch) * KEY_H;

const note = (id: string, pitch: number, start: number, duration = 1, extra: Partial<Note> = {}): Note => ({
  id,
  clip: "C",
  pitch,
  velocity: 1,
  release_velocity: 0.5,
  start,
  duration,
  muted: false,
  ...extra,
});

const colors = (light = false): GridColors => ({
  light,
  accent: [1, 0, 0, 1],
  accentDim: [0.5, 0, 0, 1],
  accentGlow: [1, 0, 1, 1],
  text: [0.9, 0.9, 0.9, 1],
  bgInset: [0.1, 0.1, 0.1, 1],
  scaleRow: [0.2, 0.2, 0.3, 1],
  scaleRootRow: [0.3, 0.2, 0.3, 1],
  scaleRootLine: [0.8, 0.5, 0.8, 0.35],
  white: [1, 1, 1, 1],
  black: [0, 0, 0, 1],
  radius: 2,
  borderWidth: 1,
  glowInner: 6,
  glowOuter: 14,
  scaleOutNote: 0.45,
});
const palette = gridPalette(colors());
/** A color as stored (float32) matches `c`. */
const near = (c: Rgba) => c.map((v) => expect.closeTo(v, 5));

function notesScene(input: Partial<NotesLayerInput> & Pick<NotesLayerInput, "notes">, p = palette) {
  const out = new InstanceList();
  const tags: SceneTag[] = [];
  buildNotesLayer(
    { ...geo, top: 0, height: 128 * KEY_H, width: 1000, selected: new Set(), multi: false, focus: "C", scale: CHROMATIC_SCALE, highlight: true, ...input },
    p,
    out,
    tags,
  );
  return tagged(out, tags);
}

describe("note grid hit testing", () => {
  const a = note("a", 60, 1);
  const b = note("b", 60, 1.5);
  const c = note("c", 64, 4, 0.01);

  it("finds the note under a point, with its zone (edges resize, short notes keep a body)", () => {
    expect(hitNote([a], new Set(), 60, y(60) + 5, geo)).toMatchObject({ note: a, zone: "body" });
    expect(hitNote([a], new Set(), 41, y(60) + 5, geo)?.zone).toBe("start");
    expect(hitNote([a], new Set(), 79, y(60) + 5, geo)?.zone).toBe("end");
    expect(hitNote([a], new Set(), 60, y(61) + 5, geo)).toBeNull();
    expect(hitNote([a], new Set(), 81, y(60) + 5, geo)).toBeNull();
    expect(hitNote([a], new Set(), 60, -1, geo)).toBeNull();
    // A very short note is still 2 px wide (as drawn).
    expect(hitNote([c], new Set(), 161.5, y(64) + 5, geo)?.note).toBe(c);
  });

  it("takes the topmost note: later over earlier, selected over unselected", () => {
    expect(hitNote([a, b], new Set(), 70, y(60) + 5, geo)?.note).toBe(b);
    expect(hitNote([a, b], new Set(["a"]), 70, y(60) + 5, geo)?.note).toBe(a);
  });

  it("uses the folded rows (a pitch's nearest displayed row)", () => {
    const rows = createPitchRows({ root: 0, kind: "Minor" }, true);
    const folded = { ...geo, rows };
    const row = rows.indexOf(60);
    expect(hitNote([a], new Set(), 60, row * KEY_H + 5, folded)?.note).toBe(a);
    expect(noteBox(a, folded)).toEqual({ x: 40, y: row * KEY_H, w: 40, h: KEY_H });
    // Every pitch maps to the row `pitchRow` gives (a hidden pitch: the nearest shown one).
    const table = rowTable(rows);
    for (let p = 0; p < 128; p++) expect(table[p]).toBe(pitchRow(p, rows));
  });

  it("picks the cursor like the old CSS did", () => {
    expect(gridCursor(null, false, false)).toBe("default");
    expect(gridCursor(null, true, true)).toBe("crosshair");
    expect(gridCursor({ zone: "body" }, false, false)).toBe("move");
    expect(gridCursor({ zone: "body" }, true, false)).toBe("ns-resize");
    expect(gridCursor({ zone: "body" }, true, true)).toBe("move");
    expect(gridCursor({ zone: "end" }, true, false)).toBe("ew-resize");
    expect(gridCursor({ zone: "start" }, false, true)).toBe("ew-resize");
  });

  it("stays fast with 10k notes", () => {
    const many = Array.from({ length: 10_000 }, (_, i) => note(`n${i}`, i % 128, (i * 0.25) % 400, 0.25));
    const t0 = performance.now();
    for (let i = 0; i < 100; i++) hitNote(many, new Set(), i * 7, (i * 13) % 1280, geo);
    expect(performance.now() - t0).toBeLessThan(500);
  });
});

describe("note grid layers", () => {
  it("draws the rows: black keys, scale tones, C and root lines, culled to the frame", () => {
    const out = new InstanceList();
    const tags: SceneTag[] = [];
    // Rows 60..71 of C minor with the highlight (the frame: 12 rows from C5 = pitch 71 down).
    buildRowsLayer(
      { ...geo, top: y(71), height: 12 * KEY_H - 1, width: 800, scale: { root: 0, kind: "Minor" }, highlight: true, lines: [] },
      palette,
      out,
      tags,
    );
    const items = tagged(out, tags);
    const rows = items.filter((i) => i.kind === "row");
    expect(rows.map((r) => r.kind === "row" && r.pitch)).toEqual([71, 70, 69, 68, 67, 66, 65, 64, 63, 62, 61, 60]);
    const row = (pitch: number) => items.find((i) => i.kind === "row" && i.pitch === pitch)!;
    expect(row(60)).toMatchObject({ tone: "root", fill: near(palette.rowRoot), y: 11 * KEY_H, w: 800, h: KEY_H });
    expect(row(62)).toMatchObject({ tone: "in", fill: near(palette.rowIn) });
    expect(row(64)).toMatchObject({ tone: "out", fill: near(palette.rowBlack) });
    // C: its line on the bottom px, the root's inset line just above it.
    const lines = items.filter((i) => i.kind === "row-line");
    expect(lines).toEqual([
      expect.objectContaining({ pitch: 60, line: "c", y: 12 * KEY_H - 1, h: 1 }),
      expect.objectContaining({ pitch: 60, line: "root", y: 12 * KEY_H - 2, h: 1, fill: near(palette.scaleRootLine) }),
    ]);
  });

  it("without a highlight only black keys are tinted; grid lines by level, rounded, culled", () => {
    const out = new InstanceList();
    const tags: SceneTag[] = [];
    const lines = [
      { beats: 0, level: "bar" as const },
      { beats: 1.01, level: "beat" as const },
      { beats: 1.25, level: "sub" as const },
      { beats: 99, level: "beat" as const },
    ];
    buildRowsLayer({ ...geo, top: y(71), height: 12 * KEY_H, width: 800, scale: CHROMATIC_SCALE, highlight: true, lines }, palette, out, tags);
    const items = tagged(out, tags);
    expect(items.filter((i) => i.kind === "row").map((r) => r.kind === "row" && r.pitch)).toEqual([70, 68, 66, 63, 61]);
    const grid = items.filter((i) => i.kind === "grid-line");
    expect(grid.map((g) => [g.x, g.fill])).toEqual([
      [0, near(palette.line.bar)],
      [40, near(palette.line.beat)],
      [50, near(palette.line.sub)],
    ]);
  });

  it("draws visible notes only, selected glows under every note, selected bodies last, opacity by velocity", () => {
    const a = note("a", 60, 1, 1, { velocity: 0 });
    const b = note("b", 62, 2);
    const far = note("far", 60, 100);
    const below = note("below", 10, 1);
    const items = notesScene({ notes: [a, b, far, below], selected: new Set(["a"]), top: y(70), height: 20 * KEY_H, highlight: false });
    expect(items.map((i) => `${i.kind}:${"id" in i ? i.id : ""}`)).toEqual(["glow:a", "glow:a", "note:b", "note:a"]);
    const drawnA = items.at(-1)!;
    expect(drawnA).toMatchObject({ x: 40, y: y(60) - y(70), w: 40, h: KEY_H, radius: 2, borderWidth: 1, opacity: expect.closeTo(0.45, 5), selected: true });
    // Selected (dark theme): lighter fill and rim, the glow's two layers (outer first).
    expect(drawnA.fill).toEqual(near([1, 0.3, 0.3, 1]));
    expect(items[0]).toMatchObject({ blur: 14, fill: near([1, 0, 1, 0.4]) });
    expect(items[1]).toMatchObject({ blur: 6, fill: near([1, 0, 1, 0.6]) });
  });

  it("light theme: darker selection, no glow", () => {
    const light = gridPalette(colors(true));
    const items = notesScene({ notes: [note("a", 60, 1)], selected: new Set(["a"]) }, light);
    expect(items).toHaveLength(1);
    expect(items[0]!.fill).toEqual(near([0.75, 0, 0, 1]));
  });

  it("muted notes are grey; out-of-scale ones dimmed unless selected; roots topped with the text color", () => {
    const scale = { root: 0, kind: "Major" as const };
    const items = notesScene({
      notes: [note("muted", 60, 1, 1, { muted: true }), note("out", 61, 2), note("outSel", 61, 3)],
      selected: new Set(["outSel"]),
      scale,
    });
    const byId = (id: string) => items.find((i) => i.kind === "note" && i.id === id)!;
    expect(byId("muted").fill).toEqual(near(grayscale(palette.accent)));
    expect(byId("muted").top).toEqual(near(grayscale(palette.text)));
    expect(byId("out").opacity).toBeCloseTo(0.45);
    expect(byId("out").top).toEqual(byId("out").border);
    expect(byId("outSel").opacity).toBe(1);
  });

  it("multi-clip: clip colors, other clips dimmed, ghosts dashed and translucent", () => {
    const green: Rgba = [0, 1, 0, 1];
    const items = notesScene({
      notes: [note("a", 60, 1), { ...note("b", 62, 2), clip: "D" }],
      ghosts: [{ key: "g", note: note("a", 60, 9) }],
      multi: true,
      focus: "C",
      clipColors: new Map([["D", green]]),
    });
    expect(items[0]).toMatchObject({ kind: "ghost", x: 360, dash: 3, fill: near([1, 0, 0, 0.35]), border: near([1, 0, 0, 0.7]) });
    const b = items.find((i) => i.kind === "note" && i.id === "b")!;
    expect(b).toMatchObject({ inactive: true, fill: near(green), border: near([0, 0.6, 0, 1]) });
    expect(b.opacity).toBeCloseTo(0.6);
  });

  it("builds 10k notes quickly (a frame's budget)", () => {
    const many = Array.from({ length: 10_000 }, (_, i) => note(`n${i}`, i % 128, (i * 0.1) % 25, 0.25));
    const out = new InstanceList();
    const input = {
      ...geo,
      top: 0,
      height: 128 * KEY_H,
      width: 1000,
      notes: many,
      selected: new Set(["n1", "n2"]),
      multi: false,
      focus: "C",
      scale: CHROMATIC_SCALE,
      highlight: false,
    };
    buildNotesLayer(input, palette, out);
    expect(out.count).toBe(10_000 + 4);
    const t0 = performance.now();
    for (let i = 0; i < 10; i++) {
      out.clear();
      buildNotesLayer(input, palette, out);
    }
    expect((performance.now() - t0) / 10).toBeLessThan(50);
  });
});
