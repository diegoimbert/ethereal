/**
 * Note stretch (base-109, owner request, Ableton-style): a bar on the piano roll's ruler
 * spans the selection: the section (section-edit) when there is one with notes, else the
 * selected notes from the earliest start to the latest end.
 *
 * - Dragging its right edge time-scales the notes about the left edge; the left edge scales
 *   about the right edge. Every start (relative to the anchor), every duration and every
 *   gap scales by the same factor: an exact proportional stretch / compress.
 * - Dragging the body moves the notes (and the section) in time.
 * - The dragged edge (or the moved start) snaps to the grid; Alt bypasses snapping.
 * - The edge never crosses the anchor, notes keep `MIN_NOTE_BEATS`, nothing goes before 0,
 *   and the clip grows when the notes run past its end (like a paste).
 *
 * Pure math; the bar is `StretchBar.tsx`. A drag is one undo gesture of `Edit.Batch`es
 * (note edits plus clip bounds), so it replicates in collab like any edit.
 */

import type { Beats, Clip, Command, Note, NoteEdit, NoteId } from "@/generated";
import { cmd } from "@/transport";
import { MIN_NOTE_BEATS, noteEdit } from "./noteEdits";
import { editSource, growCommands, type PianoRollSection } from "./section";

export type StretchHandle = "start" | "end" | "move";

/** Width in px of the resize zone at each end of the bar (the loop brace's approach). */
export const STRETCH_EDGE_PX = 6;

/** Shortest range a stretch can produce. */
export const MIN_STRETCH_BEATS: Beats = MIN_NOTE_BEATS;

export interface StretchRange {
  start: Beats;
  end: Beats;
}

export interface StretchSource extends StretchRange {
  notes: Note[];
  /** The range is the section (it follows the stretch). */
  section: boolean;
}

/** Which part of the bar a pointer `offset` px from its left edge grabs. */
export function stretchHandleAt(offset: number, width: number): StretchHandle {
  const edge = Math.min(STRETCH_EDGE_PX, width / 3);
  return offset <= edge ? "start" : offset >= width - edge ? "end" : "move";
}

/**
 * What the bar spans and stretches: the section with its notes (the selected ones inside
 * it, or all of them when none is selected), else the selected notes over their span.
 * A zero-length section (the piano roll's insert marker, an edit cursor) is not a range:
 * the bar ignores it and falls back to the selected notes. Null when there are no notes to
 * stretch.
 */
export function stretchSource(
  notes: ReadonlyArray<Note>,
  selected: ReadonlySet<NoteId>,
  section: PianoRollSection | null,
): StretchSource | null {
  if (section && section.end - section.start > 1e-9) {
    const src = editSource(notes, selected, section, null);
    return src && src.notes.length ? { ...src, section: true } : null;
  }
  const picked = notes.filter((n) => selected.has(n.id));
  if (!picked.length) return null;
  const start = Math.min(...picked.map((n) => n.start));
  const end = Math.max(...picked.map((n) => n.start + n.duration));
  return end - start > 0 ? { start, end, notes: picked, section: false } : null;
}

/**
 * The range after dragging `handle` by `rawBeats` from `range`. `snap` puts a position on
 * the grid (identity when snapping is off). Edges never cross the anchor; nothing goes
 * before 0.
 */
export function stretchRange(
  range: StretchRange,
  handle: StretchHandle,
  rawBeats: Beats,
  snap: (beats: Beats) => Beats,
): StretchRange {
  if (handle === "end") {
    return { start: range.start, end: Math.max(range.start + MIN_STRETCH_BEATS, snap(range.end + rawBeats)) };
  }
  if (handle === "start") {
    const start = Math.min(range.end - MIN_STRETCH_BEATS, Math.max(0, snap(range.start + rawBeats)));
    return { start: Math.max(0, start), end: range.end };
  }
  const d = Math.max(-range.start, snap(range.start + rawBeats) - range.start);
  return { start: range.start + d, end: range.end + d };
}

/** Map `notes` from `from` onto `to` (affine in time): starts, durations and gaps scale. */
export function stretchEdits(notes: ReadonlyArray<Note>, from: StretchRange, to: StretchRange): NoteEdit[] {
  const factor = (to.end - to.start) / (from.end - from.start);
  return notes.map((n) =>
    noteEdit(n.id, {
      start: Math.max(0, to.start + (n.start - from.start) * factor),
      duration: Math.max(MIN_NOTE_BEATS, n.duration * factor),
    }),
  );
}

/**
 * The command stretching `source` onto `to` in `clip` (the clip as it was when the drag
 * began): the note edits, plus growing the clip when the notes run past its end. `restore`
 * (a drag that grew the clip earlier) puts its original bounds back when no growth is
 * needed any more, so the gesture never leaves it longer than the result needs.
 */
export function stretchCommand(clip: Clip, source: StretchSource, to: StretchRange, restore = false): { command: Command; grows: boolean } {
  const edits = stretchEdits(source.notes, source, to);
  const end = Math.max(to.end, ...edits.map((e) => e.start! + e.duration!));
  const grow = growCommands(clip, end);
  const commands: Command[] = [cmd("Note", { type: "Edit", edits })];
  if (grow.length) commands.push(...grow);
  else if (restore) commands.push(boundsOf(clip));
  const moved = Math.abs(to.end - to.start - (source.end - source.start)) < 1e-9;
  return { command: cmd("Edit", { type: "Batch", label: moved ? "Move Notes" : "Stretch Notes", commands }), grows: grow.length > 0 };
}

/** The command setting `clip`'s playable region back to what it is now. */
function boundsOf(clip: Clip): Command {
  if (clip.looping.enabled) return cmd("Clip", { type: "SetLoop", id: clip.id, looping: clip.looping });
  return cmd("Clip", { type: "SetBounds", id: clip.id, start: clip.start, length: clip.length, offset: clip.offset });
}

/** The section after a stretch of `source` (null when the source was not a section). */
export function stretchedSection(clip: Clip, source: StretchSource, to: StretchRange): PianoRollSection | null {
  return source.section ? { clip: clip.id, start: to.start, end: to.end } : null;
}

/** ×2 / ÷2 (Ableton's buttons): scale the source's length about its start. */
export function scaledRange(source: StretchRange, factor: number): StretchRange {
  return { start: source.start, end: source.start + Math.max(MIN_STRETCH_BEATS, (source.end - source.start) * factor) };
}
