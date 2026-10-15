/**
 * Piano-roll sections and the note clipboard (section-edit, owner report).
 *
 * The section is a time range on the clip's content axis, made like the arrangement's time
 * selection, and free like it (owner request: never widened to the notes): a Shift-drag on
 * the grid (over the rows it covers: `pitches`; it also selects the notes it touches) or a
 * drag on the strip under the ruler (every pitch); a click on empty grid, a press on a note
 * or Escape clears it, ⌘A selects the clip's whole region. Its edges snap to the grid, never
 * coarser than a beat (`sectionStep`: zooming out doesn't make it jump by bars).
 *
 * Edits act on what is INSIDE the section, like the arrangement's section on its clips: the
 * notes overlapping it (on its rows), cut at its edges (a note starting before it counts from
 * the section start), the selected ones among them if any. Copy / cut / duplicate take that
 * with the section's EXACT length (empty space included); cut and delete remove only the
 * part inside (a note crossing an edge is shortened, one spanning the section is split).
 * Without a section, the selected notes, over their exact span (⌘D lands right at its end).
 *
 * - ⌘C copies; ⌘X copies then removes the source notes (no time shift).
 * - ⌘V pastes right after the section, else at the insert marker, else at the selection
 *   cursor (`time-edits/marker.ts`, in the clip; never the playhead), merging with
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
import type { GridStep } from "@/timeline";
import type { StepLabel } from "@/features/arrangement/editMath";
import { contentEnd, songToContent } from "./clipTime";
import { noteEdit } from "./noteEdits";

const EPS = 1e-9;

export interface PianoRollSection {
  clip: ClipId;
  start: Beats;
  end: Beats;
  /**
   * The pitches it covers (inclusive), like the arrangement's section covers the tracks it was
   * drawn over: a Shift-drag on the grid sets them; absent = every pitch (the strip, ⌘A).
   */
  pitches?: PitchRange;
}

/** An inclusive MIDI pitch range. */
export interface PitchRange {
  low: number;
  high: number;
}

export interface NoteClipboard {
  /** The copied range's length: a paste covers exactly this much time. */
  length: Beats;
  /** The copied section's pitches: the pasted section covers them too. */
  pitches?: PitchRange;
  /** Notes relative to the range start (release velocity is not kept: `NoteSpec` has none). */
  notes: Array<Pick<Note, "pitch" | "velocity" | "muted" | "start" | "duration"> & { clip?: ClipId }>;
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
export function sectionOf(clip: ClipId, a: Beats, b: Beats, pitches?: PitchRange): PianoRollSection | null {
  const start = Math.max(0, Math.min(a, b));
  const end = Math.max(0, Math.max(a, b));
  if (end - start <= 1e-6) return null;
  return pitches ? { clip, start, end, pitches } : { clip, start, end };
}

/** Whether `pitch` is in `section`'s pitches (always, without a pitch range). */
export function sectionHasPitch(section: Pick<PianoRollSection, "pitches">, pitch: number): boolean {
  const p = section.pitches;
  return !p || (pitch >= p.low && pitch <= p.high);
}

/** The snap step of a section's edges: the grid's, never coarser than a beat. */
export function sectionStep(step: GridStep | null): GridStep | null {
  if (!step) return null;
  if (step.kind === "bars" || step.beats > 1) return { kind: "beats", beats: 1 };
  return step;
}

/** Notes starting inside `[start, end)`. */
export function notesIn(notes: ReadonlyArray<Note>, start: Beats, end: Beats): Note[] {
  return notes.filter((n) => n.start >= start - EPS && n.start < end - EPS);
}

/** Notes overlapping `[start, end)` (any part of them inside). */
export function notesOver(notes: ReadonlyArray<Note>, start: Beats, end: Beats): Note[] {
  return notes.filter((n) => n.start < end - EPS && n.start + n.duration > start + EPS);
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
): { start: Beats; end: Beats; notes: Note[]; pitches?: PitchRange } | null {
  if (section) {
    const inside = notesOver(notes, section.start, section.end).filter((n) => sectionHasPitch(section, n.pitch));
    const picked = inside.filter((n) => selected.has(n.id));
    const r = { start: section.start, end: section.end, notes: picked.length ? picked : inside };
    return section.pitches ? { ...r, pitches: section.pitches } : r;
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

/** The clipboard of `source`: its notes relative to its start, cut at its edges. */
export function copyOf(source: { start: Beats; end: Beats; notes: ReadonlyArray<Note>; pitches?: PitchRange }): NoteClipboard {
  const { start, end } = source;
  return {
    length: end - start,
    ...(source.pitches ? { pitches: source.pitches } : {}),
    notes: [...source.notes]
      .sort((a, b) => a.start - b.start || a.pitch - b.pitch)
      .map((n) => ({
        pitch: n.pitch,
        velocity: n.velocity,
        muted: n.muted,
        clip: n.clip,
        start: Math.max(0, n.start - start),
        duration: Math.min(n.start + n.duration, end) - Math.max(n.start, start),
      })),
  };
}

/**
 * Commands removing what of `source.notes` lies inside `[source.start, source.end)`: a note
 * inside goes, one crossing an edge is shortened to the part outside, one spanning the whole
 * range keeps its head and gets a new note for its tail. Empty when there is nothing to do.
 */
export function clearCommands(source: { start: Beats; end: Beats; notes: ReadonlyArray<Note> }): Command[] {
  const { start, end } = source;
  const remove: NoteId[] = [];
  const edits: ReturnType<typeof noteEdit>[] = [];
  const tails = new Map<ClipId, NoteSpec[]>();
  const mutedTails: NoteId[] = [];
  for (const n of source.notes) {
    const nEnd = n.start + n.duration;
    const head = n.start < start - EPS;
    const tail = nEnd > end + EPS;
    if (!head && !tail) remove.push(n.id);
    else if (head && !tail) edits.push(noteEdit(n.id, { duration: start - n.start }));
    else if (!head && tail) edits.push(noteEdit(n.id, { start: end, duration: nEnd - end }));
    else {
      edits.push(noteEdit(n.id, { duration: start - n.start }));
      const spec: NoteSpec = { id: newId(), pitch: n.pitch, velocity: n.velocity, start: end, duration: nEnd - end };
      tails.set(n.clip, [...(tails.get(n.clip) ?? []), spec]);
      if (n.muted) mutedTails.push(spec.id);
    }
  }
  const commands: Command[] = [];
  if (remove.length) commands.push(cmd("Note", { type: "Remove", ids: remove }));
  if (edits.length) commands.push(cmd("Note", { type: "Edit", edits }));
  for (const [clip, notes] of tails) commands.push(cmd("Note", { type: "Add", clip, notes }));
  if (mutedTails.length) commands.push(cmd("Note", { type: "Edit", edits: mutedTails.map((id) => noteEdit(id, { muted: true })) }));
  return commands;
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
  label: StepLabel,
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
    command: commands.length ? cmd("Edit", { type: "Batch", label: label.text, key: label.key, commands }) : null,
    ids: specs.map((s) => s.id),
    section: { clip: clip.id, start: at, end: at + cb.length, ...(cb.pitches ? { pitches: cb.pitches } : {}) },
  };
}

/**
 * Where ⌘V pastes: right after the section, at the insert marker (a zero-length section),
 * else the playhead's content position.
 */
export function pasteAt(clip: Clip, section: PianoRollSection | null, cursor: Beats): Beats {
  if (section) return section.end;
  const rel = cursor - clip.start;
  const regionStart = clip.looping.enabled ? clip.looping.start : clip.offset;
  if (rel < 0) return regionStart;
  return songToContent(clip, cursor) ?? clip.offset + rel;
}

/** The clip's whole playable region (⌘A). */
export function regionOf(clip: Clip): PianoRollSection | null {
  const start = clip.looping.enabled ? clip.looping.start : clip.offset;
  return sectionOf(clip.id, start, contentEnd(clip));
}
