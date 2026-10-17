/**
 * The computer MIDI keyboard's layout (Ableton Live's): pure functions, no state.
 *
 * Keys are physical positions (`KeyboardEvent.code`), so the layout is the same on QWERTY,
 * AZERTY and QWERTZ: the home row is the white keys, the row above it the black keys.
 *
 * ```
 *   W E   T Y U   O P        C#  D#    F#  G#  A#    C#  D#
 *  A S D F G H J K L ;     C   D   E  F   G   A   B  C   D   E
 *  Z X: octave down / up   C V: velocity down / up
 * ```
 */

/** Semitone above the octave's C of each note key. */
export const NOTE_KEYS: Readonly<Record<string, number>> = {
  KeyA: 0,
  KeyW: 1,
  KeyS: 2,
  KeyE: 3,
  KeyD: 4,
  KeyF: 5,
  KeyT: 6,
  KeyG: 7,
  KeyY: 8,
  KeyH: 9,
  KeyU: 10,
  KeyJ: 11,
  KeyK: 12,
  KeyO: 13,
  KeyL: 14,
  KeyP: 15,
  Semicolon: 16,
};

export type ControlKey = "octaveDown" | "octaveUp" | "velocityDown" | "velocityUp";

export const CONTROL_KEYS: Readonly<Record<string, ControlKey>> = {
  KeyZ: "octaveDown",
  KeyX: "octaveUp",
  KeyC: "velocityDown",
  KeyV: "velocityUp",
};

/** Octaves in Ableton's naming (C3 = MIDI note 60). */
export const MIN_OCTAVE = -2;
export const MAX_OCTAVE = 8;
export const DEFAULT_OCTAVE = 3;
export const MIN_VELOCITY = 1;
export const MAX_VELOCITY = 127;
export const DEFAULT_VELOCITY = 100;
/** C/V change the velocity by this much (Ableton: 20). */
export const VELOCITY_STEP = 20;

/** Whether the computer MIDI keyboard uses this physical key (notes and controls). */
export function isKeyboardMidiKey(code: string): boolean {
  return code in NOTE_KEYS || code in CONTROL_KEYS;
}

/** MIDI note of the octave's C. */
export function octaveBase(octave: number): number {
  return 12 * (octave + 2);
}

/** MIDI note played by `code` at `octave`, or `null` (not a note key, or above 127). */
export function noteFor(code: string, octave: number): number | null {
  const semitone = NOTE_KEYS[code];
  if (semitone === undefined) return null;
  const note = octaveBase(octave) + semitone;
  return note >= 0 && note <= 127 ? note : null;
}

export function clampOctave(octave: number): number {
  return Math.min(MAX_OCTAVE, Math.max(MIN_OCTAVE, Math.round(octave)));
}

export function clampVelocity(velocity: number): number {
  return Math.min(MAX_VELOCITY, Math.max(MIN_VELOCITY, Math.round(velocity)));
}

/**
 * Velocity after one C (`-1`) or V (`+1`) press: the next multiple of 20 in that direction,
 * within 1..=127 (1, 20, 40, … 120, 127, like Ableton).
 */
export function stepVelocity(velocity: number, dir: -1 | 1): number {
  const next = dir > 0 ? (Math.floor(velocity / VELOCITY_STEP) + 1) * VELOCITY_STEP : (Math.ceil(velocity / VELOCITY_STEP) - 1) * VELOCITY_STEP;
  return clampVelocity(next);
}

/** Octaves the popover's keyboard strip shows: the played one, one below, two above. */
export const STRIP_OCTAVES = 4;

/** First octave of the strip around `octave`, kept inside the playable range. */
export function stripStart(octave: number): number {
  return Math.min(MAX_OCTAVE - STRIP_OCTAVES + 1, Math.max(MIN_OCTAVE, octave - 1));
}

/** The velocities C/V step through: 1, 20, 40, … 120, 127. */
export function velocityStops(): number[] {
  const stops = [MIN_VELOCITY];
  for (let v = VELOCITY_STEP; v < MAX_VELOCITY; v += VELOCITY_STEP) stops.push(v);
  return [...stops, MAX_VELOCITY];
}

const NAMES = ["c", "c#", "d", "d#", "e", "f", "f#", "g", "g#", "a", "a#", "b"] as const;

/** `c3` for 60 (Ableton's naming, lowercase like all the UI). */
export function noteName(note: number): string {
  return `${NAMES[((note % 12) + 12) % 12]}${Math.floor(note / 12) - 2}`;
}

/** Status bytes: note on / off on MIDI channel 1. */
export const NOTE_ON = 0x90;
export const NOTE_OFF = 0x80;
