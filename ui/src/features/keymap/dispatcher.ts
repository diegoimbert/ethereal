/**
 * Runs the actions that have no key handler of their own: palette commands (built-ins such
 * as `transport.record`, and the palette's own actions), on the chords the keymap gives them.
 *
 * One window `keydown` listener in the capture phase, before any other handler:
 * - a chord bound to such an action runs its palette command (and nothing else sees the key);
 * - a palette action's `legacy` chord (still answered by its feature's own handler) is
 *   left to that handler while the action keeps it, and blocked once the user unbinds it,
 *   unless another action now uses that chord.
 *
 * With the default keymap no built-in palette action has a chord and every legacy chord is
 * kept, so the dispatcher does nothing until the user binds something.
 */

import { isTextEntry } from "@/features/transport-bar/engine";
import { eventChords } from "./chords";
import { derivePaletteActions, paletteEntries } from "./palette";
import { registeredActions, type KeymapAction } from "./registry";
import { allActions, effectiveChords, isBound, matchesAction, useKeymapStore } from "./store";

let paused = 0;

/** Stop dispatching while the editor records a chord. Returns the undo. */
export function pauseDispatch(): () => void {
  paused++;
  let done = false;
  return () => {
    if (!done) paused--;
    done = true;
  };
}

/** Whether the dispatcher could act at all with the current keymap (cheap pre-check). */
function mayDispatch(): boolean {
  const keymap = useKeymapStore.getState().keymap;
  if (keymap.overrides.length > 0) return true;
  return registeredActions().some((a) => !a.handled && effectiveChords(a, keymap).length > 0);
}

/** Handle a keydown; returns what it did (for tests). */
export function dispatchKey(e: KeyboardEvent): "ran" | "blocked" | null {
  if (paused > 0 || e.defaultPrevented || isTextEntry(e.target)) return null;
  if (e.target instanceof Element && e.target.closest('[role="dialog"]:not(.eth-dialog--closing), [role="menu"], [role="listbox"]')) return null;
  if (eventChords(e).length === 0 || !mayDispatch()) return null;
  const entries = paletteEntries();
  derivePaletteActions(entries);
  const keymap = useKeymapStore.getState().keymap;
  const candidates = allActions().filter((a): a is KeymapAction & { palette: string } => !a.handled && a.palette !== undefined);
  for (const a of candidates) {
    if (!matchesAction(a.id, e)) continue;
    // A chord the feature's own handler answers: leave it to it.
    if (a.legacy?.some((c) => eventChords(e).includes(c))) return null;
    const entry = entries.find((x) => x.id === a.palette);
    if (!entry) return null;
    e.preventDefault();
    e.stopPropagation();
    if (!e.repeat) entry.run();
    return "ran";
  }
  // An unbound legacy chord: keep the feature's handler from answering it (unless another
  // action uses the chord now).
  if (isBound(e)) return null;
  for (const a of candidates) {
    if (!a.legacy?.length) continue;
    const chords = eventChords(e);
    if (a.legacy.some((c) => chords.includes(c)) && !effectiveChords(a, keymap).some((c) => chords.includes(c))) {
      e.preventDefault();
      e.stopPropagation();
      return "blocked";
    }
  }
  return null;
}

/** Install the dispatcher (the keymap root mounts it once). */
export function installDispatcher(): () => void {
  window.addEventListener("keydown", dispatchKey, { capture: true });
  return () => window.removeEventListener("keydown", dispatchKey, { capture: true });
}
