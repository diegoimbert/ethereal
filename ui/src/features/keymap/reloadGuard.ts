/**
 * Keeps the browser's (and the desktop webview's) reload keys from reloading the app: a stray
 * Ctrl/⌘+R or F5 dropped the session in the middle of work. Reloading stays possible from
 * the browser's own button or menu.
 *
 * A chord the keymap binds (the user can give Mod+R to an action) goes to its handler first:
 * the guard only cancels the reload after every other listener had the key.
 */

import { eventChords, type ChordEvent } from "./chords";
import { isBound } from "./store";

/** The reload chords of browsers and webviews (keymap syntax). */
const RELOAD_CHORDS = new Set(["Mod+R", "Mod+Shift+R", "Ctrl+R", "Ctrl+Shift+R", "F5", "Mod+F5", "Shift+F5", "Ctrl+F5"]);

/** Is this keydown one of the reload chords? */
export function isReloadChord(e: ChordEvent): boolean {
  return eventChords(e).some((c) => RELOAD_CHORDS.has(c));
}

const cancel = (e: KeyboardEvent) => e.preventDefault();

function guard(e: KeyboardEvent) {
  if (!isReloadChord(e)) return;
  // Bound: its handler must still see the key (handlers skip prevented events); cancel the
  // reload once the key has gone through the page (a handler that stops it prevents it).
  if (!isBound(e)) {
    e.preventDefault();
    return;
  }
  window.addEventListener("keydown", cancel, { once: true });
  // A handler that stopped the key kept it from the listener: drop it before the next key.
  setTimeout(() => window.removeEventListener("keydown", cancel), 0);
}

/** Install the guard (the keymap root mounts it once). */
export function installReloadGuard(): () => void {
  window.addEventListener("keydown", guard, { capture: true });
  return () => {
    window.removeEventListener("keydown", guard, { capture: true });
    window.removeEventListener("keydown", cancel);
  };
}
