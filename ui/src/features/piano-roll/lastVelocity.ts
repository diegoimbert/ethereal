/**
 * The velocity of a new note: that of the last note the user touched (Ableton reuses the
 * velocity of the last selected / edited note). A note is "touched" when it is pressed in
 * the grid (the last-clicked one when several are selected), pressed in the velocity lane,
 * or drawn. The note is remembered by id and its velocity read from the project when a note
 * is drawn, so a later velocity edit of it (Alt-drag, velocity lane, undo...) counts; once it
 * is gone, the velocity it had when last touched is used. Nothing touched yet: the default.
 *
 * One value for the session (all clips), like Ableton's piano roll.
 */

import type { Note, NoteId } from "@/generated";

/** Velocity of a new note before any note was touched (MIDI 100). */
export const DEFAULT_NOTE_VELOCITY = 100 / 127;

let last: { id: NoteId; velocity: number } | null = null;

/** Remember `note` as the last touched note: new notes take its velocity. */
export function touchNote(note: Pick<Note, "id" | "velocity">): void {
  last = { id: note.id, velocity: note.velocity };
}

/**
 * The velocity of a new note: the last touched note's current velocity (looked up in
 * `notes`, the project's notes by id), the one it had when touched once it is gone, else
 * `DEFAULT_NOTE_VELOCITY`.
 */
export function newNoteVelocity(notes: Readonly<Record<NoteId, Pick<Note, "velocity">>> | null | undefined): number {
  if (!last) return DEFAULT_NOTE_VELOCITY;
  return notes?.[last.id]?.velocity ?? last.velocity;
}

/** Forget the last touched note (tests). */
export function resetLastVelocity(): void {
  last = null;
}
