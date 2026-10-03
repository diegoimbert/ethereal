/**
 * Mock of `Keymap::*` (v0.3, owned by `keymap`): the user keymap kept in the mock's user
 * library, like `crates/ether-controller/src/keymap/` (`Get` → `Keymap`, the default when
 * none is stored; `Set` validates and emits `KeymapEvent::Changed`; `Reset` back to the
 * default). One per MockTransport (the mock's "user library" lasts as long as it does).
 */

import type { Keymap, KeymapCommand, ReplyValue } from "@/generated";
import { fail } from "../documentReducer";
import type { MockHost } from "./host";

export const MAX_KEYMAP_OVERRIDES = 1024;
export const MAX_CHORDS_PER_ACTION = 4;
const MAX_ACTION_ID_LEN = 128;

const MODIFIERS = ["Mod", "Ctrl", "Alt", "Shift"];
const NAMED = ["Space", "Enter", "Escape", "Backspace", "Delete", "Tab", "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Home", "End", "PageUp", "PageDown"];
const PUNCT = [",", ".", "/", ";", "'", "[", "]", "\\", "-", "=", "`"];

/** Chord syntax, as the engine checks it (CONTRACTS.md §13.12). */
export function validChord(chord: string): boolean {
  const parts = chord.split("+");
  const key = parts.pop() ?? "";
  let next = 0;
  for (const m of parts) {
    const i = MODIFIERS.indexOf(m, next);
    if (i < 0) return false;
    next = i + 1;
  }
  if (key.length === 1) return /^[A-Z0-9]$/.test(key) || PUNCT.includes(key);
  return NAMED.includes(key) || /^F([1-9]|1[0-9]|2[0-4])$/.test(key);
}

/** Why `keymap` is invalid, or null. */
export function keymapError(keymap: Keymap): string | null {
  if (keymap.overrides.length > MAX_KEYMAP_OVERRIDES) return "too many keymap overrides";
  let previous: string | null = null;
  for (const { action, chords } of keymap.overrides) {
    if (!action || action.length > MAX_ACTION_ID_LEN || /\s/.test(action)) return `invalid action id ${JSON.stringify(action)}`;
    if (previous !== null && previous >= action) return "keymap overrides must be sorted by action, one per action";
    previous = action;
    if (chords.length > MAX_CHORDS_PER_ACTION) return `too many chords for ${action}`;
    for (const [i, c] of chords.entries()) {
      if (!validChord(c)) return `invalid chord ${JSON.stringify(c)} for ${action}`;
      if (chords.indexOf(c) !== i) return `chord ${c} twice for ${action}`;
    }
  }
  return null;
}

const DEFAULT: Keymap = { preset: "Ethereal", overrides: [] };

const clone = (k: Keymap): Keymap => ({ preset: k.preset, overrides: k.overrides.map((o) => ({ action: o.action, chords: [...o.chords] })) });

export class MockKeymap {
  private stored: Keymap | null = null;

  constructor(private readonly host: Pick<MockHost, "emit">) {}

  command(c: KeymapCommand): ReplyValue {
    switch (c.type) {
      case "Get":
        return { type: "Keymap", keymap: clone(this.stored ?? DEFAULT) };
      case "Set": {
        const error = keymapError(c.keymap);
        if (error) return fail("InvalidArgument", error);
        this.stored = clone(c.keymap);
        this.changed(this.stored);
        return { type: "Unit" };
      }
      case "Reset":
        this.stored = null;
        this.changed(DEFAULT);
        return { type: "Unit" };
    }
  }

  private changed(keymap: Keymap) {
    this.host.emit({ type: "Keymap", event: { type: "Changed", keymap: clone(keymap) } });
  }
}
