/**
 * Cursor for the duration of a pointer drag: shown everywhere (whatever is under the
 * pointer) until cleared, so a resize keeps its resize cursor after leaving the handle.
 * Styles: `html[data-drag-cursor]` in theme/base.css.
 */
export function setDragCursor(cursor: string | null): void {
  const root = document.documentElement;
  if (cursor === null) {
    delete root.dataset.dragCursor;
    root.style.removeProperty("--eth-drag-cursor");
  } else {
    root.dataset.dragCursor = "";
    root.style.setProperty("--eth-drag-cursor", cursor);
  }
}
