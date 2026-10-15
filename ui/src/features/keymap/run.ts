/**
 * Run a keymap action without its chord: the command palette, menus and the desktop app's
 * native menu bar run editor actions (copy, delete, split, quantize…) through the same key
 * handlers the keyboard reaches, so there is one implementation per action (#577, #573).
 *
 * `runAction(id)` dispatches a synthetic `keydown` that carries the action id. Every
 * handler asks `matchesAction` / `firstMatch`, which answer a tagged event by its id alone
 * (its key is "Unidentified", so nothing that tests raw keys reacts). It goes to the editor
 * that had focus last among the action's scopes (an element marked `data-keymap-scope`),
 * else to the document.
 *
 * Actions with no key handler of their own (palette commands) run their palette command.
 */
import { paletteEntries } from "./palette";
import type { KeymapScope } from "./registry";
import { findAction } from "./store";
import { ACTION_TAG } from "./tag";

/** The editors focused most recently, by scope (`data-keymap-scope`). */
const lastByScope = new Map<string, HTMLElement>();
let lastScope: string | null = null;

function noteFocus(e: FocusEvent) {
  const el = e.target instanceof Element ? e.target.closest<HTMLElement>("[data-keymap-scope]") : null;
  if (!el) return;
  const scope = el.dataset.keymapScope!;
  lastByScope.set(scope, el);
  lastScope = scope;
}

/** Track which editor had focus last (the keymap root installs it once). */
export function installScopeTracker(): () => void {
  document.addEventListener("focusin", noteFocus, true);
  return () => document.removeEventListener("focusin", noteFocus, true);
}

/** Forget the focus history (tests). */
export function resetScopeTracker(): void {
  lastByScope.clear();
  lastScope = null;
}

/** Where action `scopes` run now: the last focused editor of one of them, if still mounted. */
function targetFor(scopes: ReadonlyArray<KeymapScope>): HTMLElement | null {
  const live = (s: string) => {
    const el = lastByScope.get(s);
    return el?.isConnected ? el : null;
  };
  if (lastScope && scopes.includes(lastScope as KeymapScope)) {
    const el = live(lastScope);
    if (el) return el;
  }
  for (const s of scopes) {
    const el = live(s);
    if (el) return el;
  }
  // Not focused yet this session: the first mounted editor of the scopes.
  for (const s of scopes) {
    const el = document.querySelector<HTMLElement>(`[data-keymap-scope="${s}"]`);
    if (el) return el;
  }
  return null;
}

/**
 * Run action `id` as if its chord were pressed where it applies: in the editor that holds
 * `from` (a context menu's element), else the editor focused last. Returns whether something
 * handled it (a key handler took the event, or its palette command ran).
 */
export function runAction(id: string, from?: Element | null): boolean {
  const action = findAction(id);
  if (!action) return false;
  if (!action.handled) {
    const entry = paletteEntries().find((p) => p.id === (action.palette ?? id));
    if (!entry) return false;
    entry.run();
    return true;
  }
  const global = action.scopes.includes("global");
  const own = from?.closest<HTMLElement>("[data-keymap-scope]");
  const inOwn = own?.isConnected && action.scopes.includes(own.dataset.keymapScope as KeymapScope) ? own : null;
  const target = inOwn ?? targetFor(action.scopes.filter((s) => s !== "global")) ?? (global ? document.body : null);
  if (!target) return false;
  if (target !== document.body && !target.contains(document.activeElement)) target.focus({ preventScroll: true });
  const e = new KeyboardEvent("keydown", { key: "Unidentified", bubbles: true, cancelable: true });
  Object.defineProperty(e, ACTION_TAG, { value: id });
  target.dispatchEvent(e);
  return e.defaultPrevented;
}
