// The docs' shortcuts, from the real keymap (one source of truth): the registry's built-in
// actions, the actions other features register, and the palette commands with a shortcut of
// their own. `shortcuts.test.ts` checks this list against the app's actual keymap, so a new
// shortcut can't be missing here. Pure: no stores, no engine (the docs are their own chunk).
import { create } from "zustand";
import { KEYBOARD_MIDI_ACTION } from "@/features/keyboard-midi/action";
import { formatChord, formatKeys, isMacPlatform } from "@/features/keymap/chords";
import { scopesLabel } from "@/features/keymap/groups";
import { BUILTIN_ACTIONS, type KeymapAction } from "@/features/keymap/registry";

/** Every keymap action the docs list (default "Ethereal" preset chords, plus Ableton-like ones). */
export const DOC_ACTIONS: ReadonlyArray<KeymapAction> = [...BUILTIN_ACTIONS, KEYBOARD_MIDI_ACTION];

export function docAction(id: string): KeymapAction | undefined {
  return DOC_ACTIONS.find((a) => a.id === id);
}

/** Where an action works, for the table ("" = everywhere). */
export function scopeLabel(a: KeymapAction): string {
  return scopesLabel(a) ?? "";
}

// ---- Which keys to show: macOS or Windows / Linux ----------------------------------------

export type KeyPlatform = "mac" | "pc";

const KEYS_STORAGE = "eth-docs-keys";

function initialKeys(): KeyPlatform {
  try {
    const v = localStorage.getItem(KEYS_STORAGE);
    if (v === "mac" || v === "pc") return v;
  } catch {
    // storage unavailable
  }
  return isMacPlatform() ? "mac" : "pc";
}

export const useKeyPlatform = create<{ keys: KeyPlatform }>()(() => ({ keys: initialKeys() }));

export function setKeyPlatform(keys: KeyPlatform): void {
  useKeyPlatform.setState({ keys });
  try {
    localStorage.setItem(KEYS_STORAGE, keys);
  } catch {
    // remembered for this page only
  }
}

/** A chord as it reads on `keys`' platform. */
export function chordLabel(chord: string, keys: KeyPlatform): string {
  return formatChord(chord, keys === "mac");
}

/** A chord or modifiers (`Alt`, `Mod+Shift`) as they read on `keys`' platform. */
export function keysLabel(chord: string, keys: KeyPlatform): string {
  return formatKeys(chord, keys === "mac");
}
