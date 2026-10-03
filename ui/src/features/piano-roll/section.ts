/**
 * Piano-roll sections and the note clipboard (section-edit, owner report).
 *
 * The section is a time range on the clip's content axis, made like the arrangement's time
 * selection: a marquee drag on empty grid (it also selects the notes it touches) or a drag
 * on the strip under the ruler (selects every shown note starting in it); a click on empty
 * grid, a press on a note or Escape clears it, ⌘A selects the clip's whole region.
 *
 * Copy / cut / paste / duplicate act on the "source" notes: with a section, the selected
 * notes starting inside it (every note starting inside it when none is selected), timed
 * from the section start and with the section's EXACT length (empty space before, between
 * and after the notes included; notes are cut at the section end). Without a section, the
 * selected notes, over their span rounded out to the grid.
 *
 * - ⌘C copies; ⌘X copies then removes the source notes (no time shift).
 * - ⌘V pastes right after the section, else at the insert marker, else at the playhead (in
 *   the clip), merging with
 *   the notes there; ⌘D pastes a copy right after the section without touching the
 *   clipboard. Either way the pasted range becomes the section and its notes the selection,
 *   so repeating tiles the section, gaps preserved. The clip grows when the paste runs
 *   past its end (its length, or its loop end when looping).
 *
 * The INSERT MARKER (owner request, like the arrangement's): a click on empty grid places it
 * (snapped, Alt: free) as a ZERO-LENGTH section in the same store; ⌘V then pastes there
 * instead of at the playhead, which a click never moves while playing (`setPlayStart`).
 * Copy / cut / duplicate / delete ignore it (it is not a range).
 *
 * Each edit is one undo step (an `Edit.Batch`), replicated in collab like any document edit.
 * The clipboard is per app (module state), so notes copy between clips.
 */

import { create } from "zustand";
import type { Beats, Clip, ClipId, Command, Note, NoteId, NoteSpec } from "@/generated";
import { cmd, newId } from "@/transport";
import { contentEnd, songToContent } from "./clipTime";

const EPS = 1e-9;

export interface PianoRollSection {
  clip: ClipId;
  start: Beats;
  end: Beats;
}

export interface NoteClipboard {
  /** The copied range's length: a paste covers exactly this much time. */
  length: Beats;
  /** Notes relative to the range start (release velocity is not kept: `NoteSpec` has none). */
  notes: Array<Pick<Note, "pitch" | "velocity" | "muted" | "start" | "duration">>;
}

interface SectionState {
  section: PianoRollSection | null;
  clipboard: NoteClipboard | null;
  setSection(section: PianoRollSection | null): void;
  setClipboard(clipboard: NoteClipboard | null): void;
}

export const usePianoRollSection = create<SectionState>()((set, get) => ({
  section: null,
  clipboard: null,
  setSection: (section) => {
    if (section === null && get().section === null) return;
    set({ section });
  },
  setClipboard: (clipboard) => set({ clipboard }),
}));

export function clearSection(): void {
  usePianoRollSection.getState().setSection(null);
}

/** Reset (tests). */
export function resetPianoRollSection(): void {
  usePianoRollSection.setState({ section: null, clipboard: null });
}

/** `true` if `section` is the insert marker (zero length). */
export function isSectionMarker(section: PianoRollSection | null): boolean {
  return section !== null && section.end - section.start <= 1e-6;
}

/** `section` if it is a real (non-empty) range, else null. */
export function rangeOfSection(section: PianoRollSection | null): PianoRollSection | null {
  return section && !isSectionMarker(section) ? section : null;
}

/** Place the insert marker in `clip` at content position `at` (replacing the section). */
export function placeSectionMarker(clip: ClipId, at: Beats): void {
  const b = Math.max(0, at);
  usePianoRollSection.getState().setSection({ clip, start: b, end: b });
}

/** A section of `clip` from two content positions (any order); null if empty. */
export function sectionOf(clip: ClipId, a: Beats, b: Beats): PianoRollSection | null {
  const start = Math.max(0, Math.min(a, b));
  const end = Math.max(0, Math.max(a, b));
  return end - start > 1e-6 ? { clip, start, end } : null;
}

/** Notes starting inside `[start, end)`. */
export function notesIn(notes: ReadonlyArray<Note>, start: Beats, end: Beats): Note[] {
  return notes.filter((n) => n.start >= start - EPS && n.start < end - EPS);
}

/**
 * The range and notes an edit acts on (see the module doc): the section with its selected
 * notes (or all its notes), else the selected notes over their grid-rounded span. Null when
 * there is nothing to act on.
 */
export function editSource(
  notes: ReadonlyArray<Note>,
  selected: ReadonlySet<NoteId>,
  section: PianoRollSection | null,
  step: Beats | null,
): { start: Beats; end: Beats; notes: Note[] } | null {
  if (section) {
    const inside = notesIn(notes, section.start, section.end);
    const picked = inside.filter((n) => selected.has(n.id));
    const anySelected = notes.some((n) => selected.has(n.id));
    return { start: section.start, end: section.end, notes: anySelected ? picked : inside };
  }
  const picked = notes.filter((n) => selected.has(n.id));
  if (!picked.length) return null;
  let start = Math.min(...picked.map((n) => n.start));
  let end = Math.max(...picked.map((n) => n.start + n.duration));
  if (step && step > EPS) {
    start = Math.floor(start / step + EPS) * step;
    end = Math.max(start + step, Math.ceil(end / step - EPS) * step);
  }
  return { start, end, notes: picked };
}

/** The clipboard of `source`: its notes relative to its start, cut at its end. */
export function copyOf(source: { start: Beats; end: Beats; notes: ReadonlyArray<Note> }): NoteClipboard {
  const { start, end } = source;
  return {
    length: end - start,
    notes: [...source.notes]
      .sort((a, b) => a.start - b.start || a.pitch - b.pitch)
      .map((n) => ({
        pitch: n.pitch,
        velocity: n.velocity,
        muted: n.muted,
        start: Math.max(0, n.start - start),
        duration: Math.min(n.duration, end - Math.max(n.start, start)),
      })),
  };
}

/** Commands making `clip` play up to content position `end` (none if it already does). */
export function growCommands(clip: Clip, end: Beats): Command[] {
  const current = contentEnd(clip);
  if (end <= current + EPS) return [];
  if (clip.looping.enabled) return [cmd("Clip", { type: "SetLoop", id: clip.id, looping: { ...clip.looping, end } })];
  return [cmd("Clip", { type: "SetBounds", id: clip.id, start: clip.start, length: clip.length + (end - current), offset: clip.offset })];
}

/**
 * Paste `cb` into `clip` at content position `at` (merging with the notes there), growing
 * the clip if needed: one undo step. Returns the command (null if there is nothing to do),
 * the new note ids and the pasted range.
 */
export function pasteCommand(
  clip: Clip,
  cb: NoteClipboard,
  at: Beats,
  label: string,
): { command: Command | null; ids: NoteId[]; section: PianoRollSection } {
  at = Math.max(0, at);
  const specs: NoteSpec[] = cb.notes.map((n) => ({
    id: newId(),
    pitch: n.pitch,
    velocity: n.velocity,
    start: at + n.start,
    duration: n.duration,
  }));
  const commands: Command[] = [];
  if (specs.length) commands.push(cmd("Note", { type: "Add", clip: clip.id, notes: specs }));
  const muted = specs.filter((_, i) => cb.notes[i]!.muted);
  if (muted.length) {
    const edits = muted.map((s) => ({ id: s.id, pitch: null, velocity: null, start: null, duration: null, muted: true }));
    commands.push(cmd("Note", { type: "Edit", edits }));
  }
  commands.push(...growCommands(clip, at + cb.length));
  return {
    command: commands.length ? cmd("Edit", { type: "Batch", label, commands }) : null,
    ids: specs.map((s) => s.id),
    section: { clip: clip.id, start: at, end: at + cb.length },
  };
}

/**
 * Where ⌘V pastes: right after the section, at the insert marker (a zero-length section),
 * else the playhead's content position.
 */
export function pasteAt(clip: Clip, section: PianoRollSection | null, playhead: Beats): Beats {
  if (section) return section.end;
  const rel = playhead - clip.start;
  const regionStart = clip.looping.enabled ? clip.looping.start : clip.offset;
  if (rel < 0) return regionStart;
  return songToContent(clip, playhead) ?? clip.offset + rel;
}

/** The clip's whole playable region (⌘A). */
export function regionOf(clip: Clip): PianoRollSection | null {
  const start = clip.looping.enabled ? clip.looping.start : clip.offset;
  return sectionOf(clip.id, start, contentEnd(clip));
}
