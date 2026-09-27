/** Piano-roll layout: pitch rows (127 at the top) and note rectangles in grid-local px. */

import type { Note } from "@/generated";
import { beatsToPx, type Rect, type TimelineViewport } from "@/timeline";

export const PITCHES = 128;
export const DEFAULT_KEY_HEIGHT = 12;
/** Key height range of the vertical zoom (cmd/ctrl + shift + wheel). */
export const MIN_KEY_HEIGHT = 5;
export const MAX_KEY_HEIGHT = 40;
export const KEYBOARD_WIDTH = 64;
export const VELOCITY_LANE_HEIGHT = 72;
/** Width of the resize zone at each end of a note (shrinks on short notes). */
export const EDGE_PX = 6;

const NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"] as const;
const BLACK = new Set([1, 3, 6, 8, 10]);

export const isBlackKey = (pitch: number): boolean => BLACK.has(((pitch % 12) + 12) % 12);

/** Note name with Ableton's octave numbering (middle C, MIDI 60, is "C3"). */
export function pitchName(pitch: number): string {
  return `${NAMES[((pitch % 12) + 12) % 12]}${Math.floor(pitch / 12) - 2}`;
}

export const pitchToY = (pitch: number, keyH: number): number => (PITCHES - 1 - pitch) * keyH;

/** Pitch of the row at `y` (clamped to 0..127). */
export function yToPitch(y: number, keyH: number): number {
  return Math.min(PITCHES - 1, Math.max(0, PITCHES - 1 - Math.floor(y / keyH)));
}

export function noteRect(n: Note, vp: TimelineViewport, keyH: number): Rect {
  const x0 = beatsToPx(n.start, vp);
  const y0 = pitchToY(n.pitch, keyH);
  return { x0, y0, x1: x0 + n.duration * vp.pxPerBeat, y1: y0 + keyH };
}

/** Which part of a note `px` (from the note's left edge) hits: resize edges or the body. */
export function noteHitZone(px: number, widthPx: number): "start" | "end" | "body" {
  const edge = Math.min(EDGE_PX, widthPx / 4);
  if (px >= widthPx - edge) return "end";
  if (px <= edge) return "start";
  return "body";
}
