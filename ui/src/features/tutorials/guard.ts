// While a tour runs, only the highlighted parts of its current step (when they take clicks)
// and the bubble itself take pointer and keyboard input. Everything else is stopped at the
// window, in the capture phase, before any of the app's own handlers see it.
//
// - Pointer: presses, clicks, context menus, wheel and touch outside the allowed parts are
//   stopped (and the bubble "nudges"). Pickers opened from an allowed control (a select's
//   list, a menu) are portaled elsewhere: they count as allowed while the step is interactive.
// - Keys: Escape asks to exit the tour (except inside a picker, where it closes it); Tab
//   cycles through the allowed parts; other keys only reach an allowed, focused control. The
//   app's window and document shortcut handlers never see them (stopped at <html> on the way
//   up), and the keymap dispatcher and transport keys are paused (`pauseDispatch`), unless
//   the step lets the transport keys through.

/** Set while the tour itself sends input (`closeFloating`): the guard lets it through. */
let bypass = 0;

/** Close the open popovers and menus (an outside press, as a click elsewhere would). */
export function closeFloating(doc: Document = document): void {
  bypass++;
  try {
    doc.body.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true, button: 0 }));
  } finally {
    bypass--;
  }
}

/** Pickers portaled out of the control that opened them. */
const LAYERS = '[role="listbox"], [role="menu"], [role="option"]';

const FOCUSABLE =
  'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"]), [role="tab"][aria-selected="true"]';

export interface GuardRegions {
  bubble(): Element | null;
  targets(): ReadonlyArray<Element>;
  /** The targets take clicks and keys. */
  interactive(): boolean;
  /** The transport keys (space) pass. */
  transportKeys(): boolean;
  /** Escape outside a picker. */
  onEscape(): void;
  /** A blocked press. */
  onBlocked(): void;
}

function elementOf(node: EventTarget | null): Element | null {
  if (node instanceof Element) return node;
  if (node instanceof Node) return node.parentElement;
  return null;
}

/** Whether input aimed at `node` may go through. `scroll`: a wheel (any highlighted part may scroll). */
export function isAllowed(node: EventTarget | null, r: Pick<GuardRegions, "bubble" | "targets" | "interactive">, scroll = false): boolean {
  const el = elementOf(node);
  if (!el) return false;
  if (r.bubble()?.contains(el)) return true;
  const interactive = r.interactive();
  if (!interactive && !scroll) return false;
  if (r.targets().some((t) => t.contains(el))) return true;
  return interactive && el.closest(LAYERS) !== null;
}

const POINTER_EVENTS = ["pointerdown", "mousedown", "click", "dblclick", "auxclick", "contextmenu", "touchstart"] as const;

/** Install the guard; returns the uninstall. */
export function installGuard(r: GuardRegions, win: Window = window): () => void {
  const doc = win.document;
  const stop = (e: Event) => {
    e.preventDefault();
    e.stopImmediatePropagation();
  };
  const onPointer = (e: Event) => {
    if (bypass > 0 || isAllowed(e.target, r)) return;
    stop(e);
    if (e.type === "pointerdown" || e.type === "touchstart") r.onBlocked();
  };
  const onWheel = (e: Event) => {
    if (!isAllowed(e.target, r, true)) stop(e);
  };
  const onDrag = (e: DragEvent) => {
    // No drops (a dropped file would import or open something mid-tour).
    if (isAllowed(e.target, r)) return;
    stop(e);
    if (e.dataTransfer) e.dataTransfer.dropEffect = "none";
  };
  const focusables = (): HTMLElement[] => {
    const roots: Element[] = [...(r.interactive() ? r.targets() : []), ...(r.interactive() ? [...doc.querySelectorAll(LAYERS)] : [])];
    const bubble = r.bubble();
    if (bubble) roots.push(bubble);
    const out: HTMLElement[] = [];
    for (const root of roots) {
      if (root instanceof HTMLElement && root.matches(FOCUSABLE)) out.push(root);
      for (const el of root.querySelectorAll<HTMLElement>(FOCUSABLE)) if (!out.includes(el)) out.push(el);
    }
    return out;
  };
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      // A picker closes first.
      if (elementOf(e.target)?.closest(LAYERS) && isAllowed(e.target, r)) return;
      stop(e);
      r.onEscape();
      return;
    }
    if (e.key === "Tab") {
      const list = focusables();
      if (list.length === 0) return stop(e);
      const at = list.indexOf(doc.activeElement as HTMLElement);
      const next = at < 0 ? (e.shiftKey ? list.length - 1 : 0) : (at + (e.shiftKey ? -1 : 1) + list.length) % list.length;
      stop(e);
      list[next]!.focus();
      return;
    }
    if (r.transportKeys()) return;
    if (!isAllowed(e.target, r)) stop(e);
  };
  // On the way up, past the focused control: the app's window / document shortcuts.
  const onKeyBubble = (e: KeyboardEvent) => {
    if (!r.transportKeys()) e.stopPropagation();
  };
  for (const t of POINTER_EVENTS) win.addEventListener(t, onPointer, { capture: true, passive: false });
  win.addEventListener("wheel", onWheel, { capture: true, passive: false });
  win.addEventListener("dragover", onDrag, { capture: true });
  win.addEventListener("drop", onDrag, { capture: true });
  win.addEventListener("keydown", onKeyDown, { capture: true });
  doc.documentElement.addEventListener("keydown", onKeyBubble);
  return () => {
    for (const t of POINTER_EVENTS) win.removeEventListener(t, onPointer, { capture: true });
    win.removeEventListener("wheel", onWheel, { capture: true });
    win.removeEventListener("dragover", onDrag, { capture: true });
    win.removeEventListener("drop", onDrag, { capture: true });
    win.removeEventListener("keydown", onKeyDown, { capture: true });
    doc.documentElement.removeEventListener("keydown", onKeyBubble);
  };
}
