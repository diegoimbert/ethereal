// The computer MIDI keyboard's keymap action, on its own (no stores): `keyboard.ts` registers
// it, and the user docs list it with the other shortcuts.
import type { KeymapAction } from "@/features/keymap/registry";

/** Keymap action of the on/off toggle (default `M`, like Ableton). */
export const KEYBOARD_MIDI_ACTION: KeymapAction = {
  id: "midi.computerKeyboard",
  labelKey: "keyboard_midi.action",
  group: "Transport",
  scopes: ["global"],
  chords: ["M"],
  handled: true,
  keywords: "computer midi keyboard qwerty play notes piano typing",
};
