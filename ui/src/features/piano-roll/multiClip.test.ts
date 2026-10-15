import { describe, expect, it } from "vitest";
import type { Clip, Command, Note, Project } from "@/generated";
import { cmd } from "@/transport";
import {
  bandAt,
  clipBand,
  clipShift,
  contentAt,
  loopGhosts,
  multiPasteCommand,
  outsideBands,
  songNotes,
  toClipContent,
  type MultiClipContext,
} from "./multiClip";
import { moveEdits, nudgeEdits } from "./noteEdits";
import { multiClipSpace } from "./rollSpace";
import { TempoMap } from "@/timeline";

const clip = (id: string, track: string, start: number, length: number, extra: Partial<Clip> = {}): Clip =>
  ({
    id,
    track,
    start,
    length,
    offset: 0,
    name: id,
    color: null,
    muted: false,
    looping: { enabled: false, start: 0, end: length },
    content: { type: "Midi" },
    ...extra,
  }) as unknown as Clip;
const note = (id: string, clipId: string, start: number, pitch = 60, duration = 1): Note =>
  ({ id, clip: clipId, pitch, velocity: 0.8, release_velocity: 0.5, start, duration, muted: false }) as Note;

// A at bar 2 (beat 4) on track 1, content offset 1; B at beat 16 on track 2.
const A = clip("A", "t1", 4, 8, { offset: 1 });
const B = clip("B", "t2", 16, 8);
const notes = { A: [note("a1", "A", 1), note("a2", "A", 4, 62), note("a0", "A", 0)], B: [note("b1", "B", 0, 67), note("b2", "B", 7.5, 65)] };
const of = (id: string) => notes[id as "A" | "B"] ?? [];

describe("multi-clip time mapping", () => {
  it("maps content to song by the clip's start and offset", () => {
    expect(clipShift(A)).toBe(3);
    expect(clipShift(B)).toBe(16);
    const shown = songNotes([clipBand(A), clipBand(B)], of);
    // a0 (content 0) is before A's start marker (offset 1): not shown.
    expect(shown.map((n) => [n.id, n.start])).toEqual([
      ["a1", 4],
      ["a2", 7],
      ["b1", 16],
      ["b2", 23.5],
    ]);
  });

  it("song back to content, including loop repetitions", () => {
    expect(contentAt(A, 4)).toBe(1);
    expect(contentAt(A, 11)).toBe(8);
    const L = clip("L", "t1", 32, 12, { looping: { enabled: true, start: 0, end: 4 } });
    // First pass [32, 36); then the loop repeats: song 37 plays content 1.
    expect(contentAt(L, 33)).toBe(1);
    expect(contentAt(L, 37)).toBe(1);
    expect(contentAt(L, 43.5)).toBe(3.5);
  });

  it("a looping clip's editable window is its first pass; the rest are ghosts", () => {
    const L = clip("L", "t1", 32, 10, { looping: { enabled: true, start: 0, end: 4 } });
    const band = clipBand(L);
    expect(band).toMatchObject({ start: 32, end: 42, editEnd: 36, loop: { start: 32, end: 36 } });
    const ghosts = loopGhosts([band], () => [note("n", "L", 1)]);
    expect(ghosts.map((g) => g.note.start)).toEqual([37, 41]);
    expect(songNotes([band], () => [note("n", "L", 1), note("late", "L", 5)]).map((n) => n.id)).toEqual(["n"]);
  });

  it("finds the clip under a position, preferring the given tracks", () => {
    const C = clip("C", "t1", 16, 4);
    const bands = [clipBand(A), clipBand(B), clipBand(C)];
    expect(bandAt(bands, 17, ["t1"])?.clip.id).toBe("C");
    expect(bandAt(bands, 17, ["t2", "t1"])?.clip.id).toBe("B");
    expect(bandAt(bands, 21, ["t1"])).toBeNull();
    expect(bandAt(bands, 12, ["t1", "t2"])).toBeNull();
  });

  it("shades only what is outside every clip", () => {
    expect(outsideBands([clipBand(A), clipBand(B)], 0, 32)).toEqual([
      { start: 0, end: 4 },
      { start: 12, end: 16 },
      { start: 24, end: 32 },
    ]);
  });
});

describe("multi-clip edits", () => {
  const space = multiClipSpace(A, [A, B]);
  const shown = songNotes(space.bands, of);
  const tempo = TempoMap.constant(120);

  it("moves keep every note inside its own clip (clamped, Ableton-like)", () => {
    const sel = shown.filter((n) => n.id === "a2" || n.id === "b1");
    // +6 beats would push a2 (song 7) past A's end (12): clamped to 11.9375 - 7.
    const edits = moveEdits(sel, sel[0]!, 6, 0, null, tempo, space.limits);
    expect(edits.map((e) => e.start)).toEqual([7 + 4.9375, 16 + 4.9375]);
    // Left: b1 sits at B's start, so nothing moves earlier.
    expect(nudgeEdits(sel, -1, 0, space.limits).map((e) => e.start)).toEqual([7, 16]);
  });

  it("maps edits back to each note's clip content (one command)", () => {
    const project = { notes: Object.fromEntries([...notes.A, ...notes.B].map((n) => [n.id, n])) } as unknown as Project;
    const ctx: MultiClipContext = {
      clips: new Map([
        ["A", A],
        ["B", B],
      ]),
      pending: new Map(),
    };
    const edit = cmd("Note", {
      type: "Edit",
      edits: [
        { id: "a2", pitch: 63, velocity: null, start: 8, duration: null, muted: null },
        { id: "b1", pitch: 68, velocity: null, start: 17, duration: null, muted: null },
        { id: "b2", pitch: null, velocity: 0.5, start: null, duration: null, muted: null },
      ],
    });
    const out = toClipContent(cmd("Edit", { type: "Batch", label: "Move Notes", commands: [edit] }), ctx, project);
    const inner = (out.command as { commands: Command[] }).commands[0]!.command as { edits: Array<{ id: string; start: number | null }> };
    expect(inner.edits.map((e) => [e.id, e.start])).toEqual([
      ["a2", 5],
      ["b1", 1],
      ["b2", null],
    ]);
  });

  it("splits quantize per clip and remembers duplicated notes' clips", () => {
    const project = { notes: Object.fromEntries([...notes.A, ...notes.B].map((n) => [n.id, n])) } as unknown as Project;
    const ctx: MultiClipContext = {
      clips: new Map([
        ["A", A],
        ["B", B],
      ]),
      pending: new Map(),
    };
    const q = toClipContent(cmd("Note", { type: "Quantize", clip: "A", notes: ["a1", "b1", "b2"], grid: 1, strength: 1, ends: false, swing: 0 }), ctx, project);
    expect(q.domain).toBe("Edit");
    const parts = (q.command as { commands: Command[] }).commands.map((c) => c.command as { clip: string; notes: string[] | null });
    expect(parts.map((p) => [p.clip, p.notes])).toEqual([
      ["A", ["a1"]],
      ["B", ["b1", "b2"]],
    ]);
    const all = toClipContent(cmd("Note", { type: "Quantize", clip: "A", notes: null, grid: 1, strength: 1, ends: false, swing: 0 }), ctx, project);
    expect((all.command as { commands: Command[] }).commands).toHaveLength(2);
    toClipContent(cmd("Note", { type: "Duplicate", copies: [{ from: "b1", new_id: "copy" }], offset: 1, transpose: 0 }), ctx, project);
    const e = toClipContent(
      cmd("Note", { type: "Edit", edits: [{ id: "copy", pitch: null, velocity: null, start: 18, duration: null, muted: null }] }),
      ctx,
      project,
    );
    expect((e.command as { edits: Array<{ start: number }> }).edits[0]!.start).toBe(2);
  });

  it("draws into the clip under the pointer (focused track first), nowhere outside clips", () => {
    expect(space.place(5)).toEqual({ clip: A, content: 2 });
    expect(space.place(20)).toEqual({ clip: B, content: 4 });
    expect(space.place(14)).toBeNull();
  });

  it("pastes each note into the clip under it, dropping notes over no clip", () => {
    const cb = {
      length: 16,
      notes: [
        { pitch: 60, velocity: 1, muted: false, start: 0, duration: 1, clip: "A" },
        { pitch: 61, velocity: 1, muted: true, start: 5, duration: 1, clip: "B" },
        { pitch: 62, velocity: 1, muted: false, start: 10, duration: 1, clip: "B" },
      ],
    };
    const r = multiPasteCommand(space.bands, cb, 8, "t1", { text: "paste notes", key: { id: "undo.note.paste", args: {} } });
    const commands = (r.command!.command as { commands: Command[] }).commands;
    const adds = commands.filter((c) => c.command.type === "Add").map((c) => c.command as { clip: string; notes: Array<{ start: number; pitch: number }> });
    expect(adds.map((a) => [a.clip, a.notes.map((n) => [n.pitch, n.start])])).toEqual([
      ["A", [[60, 5]]],
      ["B", [[62, 2]]],
    ]);
    expect(r.ids).toHaveLength(2);
    expect(r.section).toMatchObject({ start: 8, end: 24 });
  });
});
