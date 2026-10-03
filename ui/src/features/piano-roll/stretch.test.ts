import { describe, expect, it } from "vitest";
import type { Clip, Note } from "@/generated";
import { snapToGrid, TempoMap } from "@/timeline";
import { MIN_NOTE_BEATS } from "./noteEdits";
import {
  MIN_STRETCH_BEATS,
  scaledRange,
  stretchCommand,
  stretchEdits,
  stretchHandleAt,
  stretchRange,
  stretchSource,
  type StretchRange,
} from "./stretch";

const note = (id: string, start: number, duration: number, pitch = 60): Note =>
  ({ id, clip: "C", pitch, velocity: 0.8, release_velocity: 0.5, start, duration, muted: false }) as unknown as Note;
const tempo = TempoMap.constant(120);
const quarter = { kind: "beats" as const, beats: 1 };
const snapOn = (b: number) => snapToGrid(b, quarter, tempo);
const snapOff = (b: number) => b;
const placed = (notes: Note[], from: StretchRange, to: StretchRange) =>
  stretchEdits(notes, from, to).map((e) => [e.start, e.duration]);

describe("note stretch math", () => {
  const a = note("a", 0, 1);
  const b = note("b", 2, 1);
  const range = { start: 0, end: 4 };

  it("right edge to 8 doubles starts, durations and gaps about the left edge", () => {
    const to = stretchRange(range, "end", 4, snapOn);
    expect(to).toEqual({ start: 0, end: 8 });
    expect(placed([a, b], range, to)).toEqual([
      [0, 2],
      [4, 2],
    ]);
  });

  it("left edge scales about the right edge", () => {
    const c = note("c", 4, 1);
    const d = note("d", 6, 1);
    const from = { start: 4, end: 8 };
    const to = stretchRange(from, "start", -4, snapOn);
    expect(to).toEqual({ start: 0, end: 8 });
    expect(placed([c, d], from, to)).toEqual([
      [0, 2],
      [4, 2],
    ]);
    // Compress: the left edge to 6 halves everything toward the right edge.
    const half = stretchRange(from, "start", 2, snapOn);
    expect(placed([c, d], from, half)).toEqual([
      [6, 0.5],
      [7, 0.5],
    ]);
  });

  it("the dragged edge snaps to the grid; snap off keeps the raw position", () => {
    expect(stretchRange(range, "end", 1.3, snapOn)).toEqual({ start: 0, end: 5 });
    expect(stretchRange(range, "end", 1.3, snapOff).end).toBeCloseTo(5.3);
  });

  it("clamps: the edge never crosses the anchor, notes keep a minimum length, nothing before 0", () => {
    expect(stretchRange(range, "end", -10, snapOn)).toEqual({ start: 0, end: MIN_STRETCH_BEATS });
    expect(stretchRange(range, "start", 10, snapOn)).toEqual({ start: 4 - MIN_STRETCH_BEATS, end: 4 });
    expect(stretchRange({ start: 2, end: 4 }, "start", -5, snapOn)).toEqual({ start: 0, end: 4 });
    const tiny = stretchEdits([a, b], range, { start: 0, end: MIN_STRETCH_BEATS });
    for (const e of tiny) expect(e.duration).toBeGreaterThanOrEqual(MIN_NOTE_BEATS);
    for (const e of tiny) expect(e.start).toBeGreaterThanOrEqual(0);
  });

  it("the body moves the range (snapped, never before 0)", () => {
    expect(stretchRange({ start: 1, end: 3 }, "move", 2.2, snapOn)).toEqual({ start: 3, end: 5 });
    expect(stretchRange({ start: 1, end: 3 }, "move", -5, snapOn)).toEqual({ start: 0, end: 2 });
    expect(placed([note("x", 1, 1)], { start: 1, end: 3 }, { start: 3, end: 5 })).toEqual([[3, 1]]);
  });

  it("×2 / ÷2 scale the length about the start", () => {
    expect(scaledRange({ start: 1, end: 3 }, 2)).toEqual({ start: 1, end: 5 });
    expect(scaledRange({ start: 1, end: 3 }, 0.5)).toEqual({ start: 1, end: 2 });
  });

  it("hit-tests the edges and the body", () => {
    expect(stretchHandleAt(2, 200)).toBe("start");
    expect(stretchHandleAt(100, 200)).toBe("move");
    expect(stretchHandleAt(198, 200)).toBe("end");
    // A narrow bar keeps a body to grab.
    expect(stretchHandleAt(6, 12)).toBe("move");
  });

  it("the source: the section's notes, else the selected notes over their span", () => {
    const notes = [a, b, note("z", 5, 2)];
    expect(stretchSource(notes, new Set(["a", "z"]), null)).toMatchObject({ start: 0, end: 7, section: false });
    const sec = stretchSource(notes, new Set(), { clip: "C", start: 0, end: 4 });
    expect(sec).toMatchObject({ start: 0, end: 4, section: true });
    expect(sec!.notes.map((n) => n.id)).toEqual(["a", "b"]);
    expect(stretchSource(notes, new Set(), { clip: "C", start: 8, end: 10 })).toBeNull();
    expect(stretchSource(notes, new Set(), null)).toBeNull();
    // A zero-length section (the insert marker) is ignored: the selected notes span the bar.
    expect(stretchSource(notes, new Set(), { clip: "C", start: 2, end: 2 })).toBeNull();
    expect(stretchSource(notes, new Set(["a", "b"]), { clip: "C", start: 2, end: 2 })).toMatchObject({ start: 0, end: 3, section: false });
  });

  it("the command grows the clip past its end, and restores it once no longer needed", () => {
    const clip = { id: "C", start: 0, length: 4, offset: 0, looping: { enabled: false, start: 0, end: 4 } } as unknown as Clip;
    const source = { start: 0, end: 4, notes: [a, b], section: true };
    const grown = stretchCommand(clip, source, { start: 0, end: 8 });
    expect(grown.grows).toBe(true);
    const kinds = (c: typeof grown.command) => (c.domain === "Edit" && c.command.type === "Batch" ? c.command.commands.map((x) => `${x.domain}.${x.command.type}`) : []);
    expect(kinds(grown.command)).toEqual(["Note.Edit", "Clip.SetBounds"]);
    const back = stretchCommand(clip, source, { start: 0, end: 3 }, true);
    expect(back.grows).toBe(false);
    expect(kinds(back.command)).toEqual(["Note.Edit", "Clip.SetBounds"]);
    expect(kinds(stretchCommand(clip, source, { start: 0, end: 3 }).command)).toEqual(["Note.Edit"]);
  });
});
