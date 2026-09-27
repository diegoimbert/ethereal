/** Piano-roll layout: pitch rows (127 at the top) and note rectangles in grid-local px. */

import type { Note } from "@/generated";
import type { MusicalScale } from "@/generated";
import { CHROMATIC_SCALE, getPitchClass, isNoteInScale, ROOT_NOTES } from "@/domain/scales";
import { beatsToPx, type Rect, type TimelineViewport } from "@/timeline";

export const PITCHES = 128;
export const DEFAULT_KEY_HEIGHT = 12;
export const KEYBOARD_WIDTH = 64;
export const VELOCITY_LANE_HEIGHT = 72;
/** Width of the resize zone at each end of a note (shrinks on short notes). */
export const EDGE_PX = 6;

const BLACK = new Set([1, 3, 6, 8, 10]);

export const isBlackKey = (pitch: number): boolean => BLACK.has(getPitchClass(pitch));

/** Note name with Ableton's octave numbering (middle C, MIDI 60, is "C3"). */
export function pitchName(pitch: number): string {
  return `${ROOT_NOTES[getPitchClass(pitch)]}${Math.floor(pitch / 12) - 2}`;
}

/** Descending visible MIDI pitches. All layout and hit testing use this same mapping. */
export function createPitchRows(scale: MusicalScale = CHROMATIC_SCALE, only = false): readonly number[] {
  return Array.from({ length: PITCHES }, (_, i) => PITCHES - 1 - i)
    .filter((pitch) => !only || isNoteInScale(pitch, scale.root, scale.kind));
}
export const ALL_PITCH_ROWS = createPitchRows();

/** Nearest visible row, also used to preserve the viewport when folding rows. */
export function pitchRow(pitch: number, rows: readonly number[]): number {
  let nearest = 0;
  for (let i = 1; i < rows.length; i++) {
    if (Math.abs(rows[i]! - pitch) < Math.abs(rows[nearest]! - pitch)) nearest = i;
  }
  return nearest;
}
export const pitchToY = (pitch: number, keyH: number, rows = ALL_PITCH_ROWS): number => pitchRow(pitch, rows) * keyH;

/** Pitch of the row at `y` (clamped to 0..127). */
export function yToPitch(y: number, keyH: number, rows = ALL_PITCH_ROWS): number {
  return rows[Math.min(rows.length - 1, Math.max(0, Math.floor(y / keyH)))]!;
}

export function noteRect(n: Note, vp: TimelineViewport, keyH: number, rows = ALL_PITCH_ROWS): Rect {
  const x0 = beatsToPx(n.start, vp);
  const y0 = pitchToY(n.pitch, keyH, rows);
  return { x0, y0, x1: x0 + n.duration * vp.pxPerBeat, y1: y0 + keyH };
}

/** Which part of a note `px` (from the note's left edge) hits: resize edges or the body. */
export function noteHitZone(px: number, widthPx: number): "start" | "end" | "body" {
  const edge = Math.min(EDGE_PX, widthPx / 4);
  if (px >= widthPx - edge) return "end";
  if (px <= edge) return "start";
  return "body";
}
