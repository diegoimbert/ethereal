/**
 * The arrangement's clip drag in progress (move, resize, slip), if any (#593).
 *
 * Escape cancels it (preview dropped, nothing sent, the selection kept): browsers don't
 * turn Escape into a `pointercancel`, and the arrangement's own Escape (deselect) must not
 * run under a drag. While a drag runs, the arrangement ignores its edit keys (⌫ would
 * delete the clips being dragged, and the drop would then fail).
 */

let current: object | null = null;

/** Keys that only change a drag's mode (copy, snapping): they still reach the drag. */
export const MODIFIER_KEYS: ReadonlySet<string> = new Set(["Alt", "Shift", "Meta", "Control"]);

/** A clip drag is in progress. */
export function clipDragActive(): boolean {
  return current !== null;
}

/**
 * Register a clip drag: until the returned release runs, Escape calls `cancel` (and goes
 * nowhere else) and `clipDragActive()` is true.
 */
export function holdClipDrag(cancel: () => void): () => void {
  const entry = {};
  const onKey = (e: KeyboardEvent) => {
    if (e.key !== "Escape") return;
    // Window capture: before the arrangement (deselect) or a dialog sees it.
    e.preventDefault();
    e.stopPropagation();
    cancel();
  };
  window.addEventListener("keydown", onKey, true);
  current = entry;
  return () => {
    window.removeEventListener("keydown", onKey, true);
    if (current === entry) current = null;
  };
}
