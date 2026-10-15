/**
 * Pure note-editing math: turns a drag (delta beats / pitch / velocity) over the notes at
 * drag start into `NoteEdit`s. Always computed from the *original* notes, so repeated
 * pointer moves inside one gesture never accumulate error.
 */

import type { Beats, Note, NoteEdit, NoteId, NoteSpec } from "@/generated";
import { snapDelta, snapToGrid, type GridStep, type TempoMap } from "@/timeline";

/** Shortest note a resize can produce (a 1/64 note). */
export const MIN_NOTE_BEATS: Beats = 1 / 16;
/** Lowest velocity a drag can set (MIDI 1/127; 0 would be a note-off). */
export const MIN_VELOCITY = 1 / 127;
export const MAX_PITCH = 127;

const clamp = (x: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, x));

export function noteEdit(id: NoteId, fields: Partial<Omit<NoteEdit, "id">>): NoteEdit {
  return {
    id,
    pitch: fields.pitch ?? null,
    velocity: fields.velocity ?? null,
    start: fields.start ?? null,
    duration: fields.duration ?? null,
    muted: fields.muted ?? null,
  };
}

/**
 * Where a note's start may move to (inclusive), e.g. its clip's window in the multi-clip
 * editor (`multiClip.ts`); `null` = anywhere from 0.
 */
export type MoveLimits = (note: Note) => readonly [Beats, Beats] | null;

/**
 * The clamped (beats, pitch) deltas that keep every note in range (start >= 0, inside its
 * `limits`, pitch 0..127).
 */
export function clampMove(notes: ReadonlyArray<Note>, dBeats: Beats, dPitch: number, limits?: MoveLimits): { dBeats: Beats; dPitch: number } {
  if (notes.length === 0) return { dBeats, dPitch };
  let lo = -Infinity;
  let hi = Infinity;
  let minPitch = MAX_PITCH;
  let maxPitch = 0;
  for (const n of notes) {
    const [min, max] = limits?.(n) ?? [0, Infinity];
    lo = Math.max(lo, Math.max(0, min) - n.start);
    hi = Math.min(hi, max - n.start);
    minPitch = Math.min(minPitch, n.pitch);
    maxPitch = Math.max(maxPitch, n.pitch);
  }
  return {
    dBeats: Math.max(lo, Math.min(hi, dBeats)),
    dPitch: clamp(Math.round(dPitch), -minPitch, MAX_PITCH - maxPitch),
  };
}

/**
 * Move `notes` by a raw drag delta. The time delta is snapped so that `anchor` (the note
 * under the pointer) lands on the grid (`step = null` = no snapping).
 */
export function moveEdits(
  notes: ReadonlyArray<Note>,
  anchor: Note,
  rawBeats: Beats,
  rawPitch: number,
  step: GridStep | null,
  tempo: TempoMap,
  limits?: MoveLimits,
): NoteEdit[] {
  const snapped = snapDelta(anchor.start, rawBeats, step, tempo);
  const { dBeats, dPitch } = clampMove(notes, snapped, rawPitch, limits);
  return notes.map((n) => noteEdit(n.id, { start: Math.max(0, n.start + dBeats), pitch: n.pitch + dPitch }));
}

/** Move by exact deltas (keyboard nudges), clamped to the valid range. */
export function nudgeEdits(notes: ReadonlyArray<Note>, dBeats: Beats, dPitch: number, limits?: MoveLimits): NoteEdit[] {
  const d = clampMove(notes, dBeats, dPitch, limits);
  return notes.map((n) => noteEdit(n.id, { start: Math.max(0, n.start + d.dBeats), pitch: n.pitch + d.dPitch }));
}

export type ResizeEdge = "start" | "end";

/**
 * Resize `notes` by a raw drag delta on one edge. The anchor's dragged edge snaps to the
 * grid and every note gets the same delta, never shorter than `MIN_NOTE_BEATS`.
 */
export function resizeEdits(notes: ReadonlyArray<Note>, anchor: Note, edge: ResizeEdge, rawBeats: Beats, step: GridStep | null, tempo: TempoMap): NoteEdit[] {
  if (edge === "end") {
    const anchorEnd = anchor.start + anchor.duration;
    const end = snapToGrid(anchorEnd + rawBeats, step, tempo);
    const d = end - anchorEnd;
    return notes.map((n) => noteEdit(n.id, { duration: Math.max(MIN_NOTE_BEATS, n.duration + d) }));
  }
  const start = snapToGrid(anchor.start + rawBeats, step, tempo);
  const d = start - anchor.start;
  return notes.map((n) => {
    const end = n.start + n.duration;
    const s = clamp(n.start + d, 0, end - MIN_NOTE_BEATS);
    return noteEdit(n.id, { start: s, duration: end - s });
  });
}

/** Offset every note's velocity by `dv` (clamped to `MIN_VELOCITY..1`). */
export function velocityEdits(notes: ReadonlyArray<Note>, dv: number): NoteEdit[] {
  return notes.map((n) => noteEdit(n.id, { velocity: clamp(n.velocity + dv, MIN_VELOCITY, 1) }));
}

/** Vertical px for a full-range (0..1) velocity drag on a note (Alt-drag; Ableton-like). */
export const VELOCITY_DRAG_PX = 180;
/** Velocity drag speed while Shift is held (fine control). */
export const VELOCITY_FINE = 0.1;

/**
 * The velocity offset of an Alt-drag on a note, fed the pointer's vertical offset from the
 * press (px, down = positive) and whether Shift is held. Up = louder. Movement is
 * accumulated, so pressing or releasing Shift mid-drag never makes the value jump.
 */
export function velocityDrag(): (dy: number, fine: boolean) => number {
  let lastDy = 0;
  let dv = 0;
  return (dy, fine) => {
    dv += (-(dy - lastDy) / VELOCITY_DRAG_PX) * (fine ? VELOCITY_FINE : 1);
    lastDy = dy;
    return dv;
  };
}

/** MIDI velocity (1..127) of a normalized velocity, as shown to the user. */
export const midiVelocity = (v: number): number => Math.round(clamp(v, 0, 1) * 127);

/** A new note at (`pitch`, `start`) with `duration`, clamped to the valid range. */
export function newNote(id: NoteId, pitch: number, start: Beats, duration: Beats, velocity = 100 / 127): NoteSpec {
  return {
    id,
    pitch: clamp(Math.round(pitch), 0, MAX_PITCH),
    velocity,
    start: Math.max(0, start),
    duration: Math.max(MIN_NOTE_BEATS, duration),
  };
}
