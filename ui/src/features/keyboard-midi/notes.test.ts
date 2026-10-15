import { describe, expect, it } from "vitest";
import { CONTROL_KEYS, isKeyboardMidiKey, MAX_OCTAVE, MIN_OCTAVE, NOTE_KEYS, noteFor, noteName, octaveBase, stepVelocity, clampOctave } from "./notes";

describe("computer keyboard layout", () => {
  it("maps the home row and the row above to a chromatic octave and a half (by key code)", () => {
    const row = ["KeyA", "KeyW", "KeyS", "KeyE", "KeyD", "KeyF", "KeyT", "KeyG", "KeyY", "KeyH", "KeyU", "KeyJ", "KeyK", "KeyO", "KeyL", "KeyP", "Semicolon"];
    expect(row.map((c) => noteFor(c, 3))).toEqual(Array.from({ length: 17 }, (_, i) => 60 + i));
    expect(Object.keys(NOTE_KEYS).sort()).toEqual([...row].sort());
  });

  it("uses physical keys, so AZERTY / QWERTZ letters don't matter", () => {
    // AZERTY's "Q" key sits where QWERTY's "A" is: its code is still KeyA.
    expect(noteFor("KeyA", 3)).toBe(60);
    // QWERTZ's "Z" key is KeyY (G#), its "Y" key is KeyZ (octave down, not a note).
    expect(noteFor("KeyY", 3)).toBe(68);
    expect(noteFor("KeyZ", 3)).toBeNull();
    expect(CONTROL_KEYS.KeyZ).toBe("octaveDown");
  });

  it("ignores other keys", () => {
    for (const code of ["KeyB", "KeyM", "Space", "Digit1", "Quote", "KeyQ"]) {
      expect(noteFor(code, 3)).toBeNull();
      expect(isKeyboardMidiKey(code)).toBe(false);
    }
    for (const code of ["KeyA", "Semicolon", "KeyZ", "KeyX", "KeyC", "KeyV"]) expect(isKeyboardMidiKey(code)).toBe(true);
  });

  it("names octaves like Ableton (C3 = 60) and stays within MIDI", () => {
    expect(octaveBase(3)).toBe(60);
    expect(noteName(60)).toBe("C3");
    expect(noteName(0)).toBe("C-2");
    expect(noteName(61)).toBe("C#3");
    expect(noteFor("KeyA", MIN_OCTAVE)).toBe(0);
    // Top octave: C8 = 120 up to G8 = 127 play, notes above 127 don't.
    expect(noteFor("KeyG", MAX_OCTAVE)).toBe(127);
    expect(noteFor("KeyY", MAX_OCTAVE)).toBeNull();
    expect(clampOctave(MAX_OCTAVE + 1)).toBe(MAX_OCTAVE);
    expect(clampOctave(MIN_OCTAVE - 1)).toBe(MIN_OCTAVE);
  });

  it("steps velocity through 1, 20, 40 … 120, 127", () => {
    expect(stepVelocity(100, 1)).toBe(120);
    expect(stepVelocity(120, 1)).toBe(127);
    expect(stepVelocity(127, 1)).toBe(127);
    expect(stepVelocity(127, -1)).toBe(120);
    expect(stepVelocity(100, -1)).toBe(80);
    expect(stepVelocity(90, -1)).toBe(80);
    expect(stepVelocity(20, -1)).toBe(1);
    expect(stepVelocity(1, -1)).toBe(1);
    expect(stepVelocity(1, 1)).toBe(20);
  });
});
