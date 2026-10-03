/**
 * Conflicts: two actions bound to the same chord where both can hear it (their scopes
 * overlap; `global` overlaps everything). The editor shows them on both rows.
 */

import type { Keymap } from "@/generated";
import { isMacPlatform, normalizeChord } from "./chords";
import type { KeymapAction, KeymapScope } from "./registry";
import { effectiveChords } from "./store";

export interface Conflict {
  /** The chord, normalized for this platform. */
  chord: string;
  /** The other actions bound to it. */
  with: KeymapAction[];
}

export function scopesOverlap(a: ReadonlyArray<KeymapScope>, b: ReadonlyArray<KeymapScope>): boolean {
  return a.includes("global") || b.includes("global") || a.some((s) => b.includes(s));
}

/** action id → its conflicts (only actions that have one). */
export function findConflicts(actions: ReadonlyArray<KeymapAction>, keymap: Keymap, mac = isMacPlatform()): Map<string, Conflict[]> {
  const byChord = new Map<string, KeymapAction[]>();
  for (const a of actions) {
    for (const c of new Set(effectiveChords(a, keymap).map((x) => normalizeChord(x, mac)))) {
      const list = byChord.get(c) ?? [];
      list.push(a);
      byChord.set(c, list);
    }
  }
  const out = new Map<string, Conflict[]>();
  for (const [chord, list] of byChord) {
    if (list.length < 2) continue;
    for (const a of list) {
      const others = list.filter((b) => b !== a && scopesOverlap(a.scopes, b.scopes));
      if (others.length === 0) continue;
      const mine = out.get(a.id) ?? [];
      mine.push({ chord, with: others });
      out.set(a.id, mine);
    }
  }
  return out;
}

/** The actions (other than `self`) that would conflict with `self` on `chord`. */
export function conflictsFor(self: KeymapAction, chord: string, actions: ReadonlyArray<KeymapAction>, keymap: Keymap, mac = isMacPlatform()): KeymapAction[] {
  const c = normalizeChord(chord, mac);
  return actions.filter(
    (a) => a.id !== self.id && scopesOverlap(a.scopes, self.scopes) && effectiveChords(a, keymap).some((x) => normalizeChord(x, mac) === c),
  );
}
