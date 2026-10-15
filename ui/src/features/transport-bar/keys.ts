/**
 * Space always plays / stops (transport-keys, owner rule): "no matter where we are in the
 * app, unless we are inside a text box". In a web app a clicked button, checkbox or select
 * keeps the focus, and Space would then "press" it instead of playing.
 *
 * One window `keydown` listener in the CAPTURE phase (before any focused element sees the
 * key) answers the transport actions (`transport.play`, default Space: play / stop back to the
 * start; `transport.stopToSelection`, default Shift+Space: play / stop in place; both
 * rebindable in the keymap) and swallows the key
 * (`preventDefault` + `stopPropagation`, and the matching `keyup`, where buttons and
 * checkboxes activate), so no control is ever activated by them.
 *
 * The rule (`keepsTransportKey`): the ONLY exception is text entry: `<textarea>`,
 * `contenteditable`, `[role="textbox"]`, `<input>` of a text-like type (text, search, email,
 * url, tel, password, number, date / time types; an editable combobox is such an input), so
 * typing a space works. Plus the keymap editor while it records a chord (`dispatchPaused`).
 * Everything else plays / stops on Space: buttons, toggles, checkboxes, radios, sliders and
 * knobs, selects (open or closed), list rows (rack chains), menus (the menu stays open; Enter
 * picks, Escape closes), dialogs, device cards. Enter still activates the focused control.
 * The target is the deepest one (`composedPath()[0]`), so a text field inside an open shadow
 * root counts too.
 *
 * Also: a mouse click doesn't leave a button focused (`blurAfterPointer`), the convention
 * for toolbar-like controls; keyboard focus (Tab) keeps its `:focus-visible` ring.
 */

import { firstMatch, type ChordEvent } from "@/features/keymap";

/** `<input>` types that take typed text (a space is a character there). */
const TEXT_INPUT_TYPES = new Set(["", "text", "search", "email", "url", "tel", "password", "number", "date", "datetime-local", "month", "time", "week"]);

/** `true` if `target` takes typed text (a space must reach it). */
export function isTypingTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  // jsdom has no `isContentEditable`; browsers answer above.
  if (
    (target.isContentEditable as boolean | undefined) === undefined &&
    target.closest('[contenteditable=""], [contenteditable="true"], [contenteditable="plaintext-only"]')
  )
    return true;
  if (target.getAttribute("role") === "textbox" && target.getAttribute("aria-readonly") !== "true") return true;
  if (target instanceof HTMLTextAreaElement) return !target.readOnly && !target.disabled;
  if (target instanceof HTMLInputElement) return TEXT_INPUT_TYPES.has(target.type.toLowerCase()) && !target.readOnly && !target.disabled;
  return false;
}

/** `true` if the focused `target` keeps a transport key (Space) instead of the transport: text entry only. */
export function keepsTransportKey(target: EventTarget | null): boolean {
  return isTypingTarget(target);
}

/** The element the key really went to: inside an open shadow root, its focused descendant. */
function deepTarget(e: { target: EventTarget | null; composedPath?: () => EventTarget[] }): EventTarget | null {
  return e.composedPath?.()[0] ?? e.target;
}

export type TransportKeyAction = "transport.play" | "transport.stopToSelection";

const TRANSPORT_ACTIONS: ReadonlyArray<TransportKeyAction> = ["transport.stopToSelection", "transport.play"];

/** The transport action a key event triggers per the keymap, or null. */
export function transportActionFor(e: ChordEvent): TransportKeyAction | null {
  return firstMatch(TRANSPORT_ACTIONS, e);
}

/** Should this key event go to the transport? (the routing rule, without side effects) */
export function routeTransportKey(
  e: ChordEvent & { target: EventTarget | null; defaultPrevented?: boolean; composedPath?: () => EventTarget[] },
): TransportKeyAction | null {
  if (e.defaultPrevented) return null;
  const action = transportActionFor(e);
  if (!action || keepsTransportKey(deepTarget(e))) return null;
  return action;
}

export interface TransportKeyHandlers {
  /** False while the transport can't act (no engine / project): keys then behave normally. */
  enabled(): boolean;
  /** Paused (the keymap editor records a chord): keys go to it. */
  paused(): boolean;
  run(action: TransportKeyAction): void;
}

/**
 * Install the capture-phase handler on `win`. Returns the uninstall. Auto-repeats are
 * swallowed without running the action again.
 */
export function installTransportKeys(handlers: TransportKeyHandlers, win: Window = window): () => void {
  // Codes whose keydown we took: their keyup is swallowed too (buttons click on keyup).
  const taken = new Set<string>();
  const onKeyDown = (e: KeyboardEvent) => {
    if (handlers.paused() || !handlers.enabled()) return;
    const action = routeTransportKey(e);
    if (!action) return;
    e.preventDefault();
    e.stopPropagation();
    taken.add(e.code || e.key);
    if (!e.repeat) handlers.run(action);
  };
  const onKeyUp = (e: KeyboardEvent) => {
    if (!taken.delete(e.code || e.key)) return;
    e.preventDefault();
    e.stopPropagation();
  };
  win.addEventListener("keydown", onKeyDown, { capture: true });
  win.addEventListener("keyup", onKeyUp, { capture: true });
  return () => {
    win.removeEventListener("keydown", onKeyDown, { capture: true });
    win.removeEventListener("keyup", onKeyUp, { capture: true });
  };
}

/** Controls a pointer click must not leave focused (the keyboard still focuses them). */
const CLICK_CONTROLS = 'button, [role="button"], input[type="checkbox"], input[type="radio"], [role="switch"]';

/** Where a click keeps the focus on the control (focus management, popups). */
const KEEP_FOCUS = '[role="dialog"], [role="menu"], [role="listbox"], [aria-haspopup]:not([aria-haspopup="false"]), [role="combobox"], [data-keep-focus]';

/**
 * After a mouse / pen / touch click on a button-like control, hand the focus to the
 * control's nearest focusable container (the arrangement, piano roll, ... so their
 * shortcuts keep working), else blur it; not in dialogs and menus (their focus management
 * needs it) nor on popup triggers, nor when the click moved the focus elsewhere. Returns
 * the uninstall.
 */
export function blurAfterPointer(win: Window = window): () => void {
  const onClick = (e: MouseEvent) => {
    // `detail === 0`: a keyboard-generated click (Enter / Space on a focused control).
    if (e.detail === 0) return;
    const el = e.target instanceof Element ? e.target.closest<HTMLElement>(CLICK_CONTROLS) : null;
    if (!el || el.closest(KEEP_FOCUS)) return;
    if (el.ownerDocument.activeElement !== el) return;
    const container = el.parentElement?.closest<HTMLElement>("[tabindex]");
    if (container) container.focus({ preventScroll: true });
    else el.blur();
  };
  win.addEventListener("click", onClick);
  return () => win.removeEventListener("click", onClick);
}
