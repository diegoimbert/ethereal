/**
 * In the browser the project saves itself `AUTOSAVE_MS` after the last edit, so closing
 * the tab (or switching away from it, which may be the last chance on mobile) within that wait
 * lost the edit. When the page is hidden or unloaded with unsaved changes, save at once.
 * A closed page's engine dies with it, so on `pagehide` the page also waits (`wait`, from a
 * microtask: the save reaches the engine in one) for the save to be written.
 * Returns the unsubscribe.
 */
export function bindSaveOnHide(dirty: () => boolean, save: () => void, wait?: () => void, doc: Document = document, win: Window = window): () => void {
  const flush = (closing: boolean) => {
    if (!dirty()) return;
    save();
    if (closing && wait) queueMicrotask(wait);
  };
  // Kept in the back/forward cache (`persisted`), the engine lives on: no need to wait.
  const onHide = (e: Event) => flush(!(e as PageTransitionEvent).persisted);
  const onVisibility = () => {
    if (doc.visibilityState === "hidden") flush(false);
  };
  doc.addEventListener("visibilitychange", onVisibility);
  win.addEventListener("pagehide", onHide);
  return () => {
    doc.removeEventListener("visibilitychange", onVisibility);
    win.removeEventListener("pagehide", onHide);
  };
}
