import { useEffect, type RefObject } from "react";

/** How long a newly shown panel may take to render its search field (engine replies). */
const SEARCH_FOCUS_WAIT_MS = 2000;

/** What keeps the focus when clicked inside a panel (its own text entry, menus, dialogs). */
const KEEPS_FOCUS = 'input, textarea, select, [contenteditable="true"], [role="combobox"], [role="listbox"], [role="menu"], [role="dialog"]';

/**
 * Focuses the search field of the left pane's panel whenever a panel is shown (`shown`: the
 * open tab, `null` when closed), as soon as the panel renders one, and again after any click
 * inside the panel (a row, empty space...; not on another field, a select or a menu): typing
 * searches at once and the panel's arrow keys work from the field.
 */
export function useFocusPanelSearch(pane: RefObject<HTMLElement | null>, shown: string | null) {
  useEffect(() => {
    const root = pane.current;
    if (!shown || !root) return;
    const tryFocus = () => {
      const input = root.querySelector<HTMLInputElement>('input[type="search"]:not(:disabled)');
      if (!input) return false;
      input.focus();
      return true;
    };
    if (tryFocus()) return;
    const observer = new MutationObserver(() => tryFocus() && stop());
    const timer = setTimeout(() => stop(), SEARCH_FOCUS_WAIT_MS);
    const stop = () => {
      observer.disconnect();
      clearTimeout(timer);
    };
    observer.observe(root, { childList: true, subtree: true, attributes: true, attributeFilter: ["disabled"] });
    return stop;
  }, [pane, shown]);
}

/** The click half of `useFocusPanelSearch`: back to the panel's search after a click in it. */
export function useClickFocusesPanelSearch(pane: RefObject<HTMLElement | null>, shown: string | null) {
  useEffect(() => {
    const root = pane.current;
    if (!shown || !root) return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const onClick = (e: MouseEvent) => {
      const target = e.target instanceof Element ? e.target : null;
      if (!target || target.closest(KEEPS_FOCUS) || target.closest('[aria-haspopup]:not([aria-haspopup="false"])')) return;
      // Selecting text (a name, a message) keeps it selected.
      if (window.getSelection()?.toString()) return;
      // After the click's own handlers (React's run above this listener), unless they moved
      // the focus to a field or a popup of their own (a rename field, a dialog).
      clearTimeout(timer);
      timer = setTimeout(() => {
        const search = root.querySelector<HTMLInputElement>('input[type="search"]:not(:disabled)');
        const active = document.activeElement;
        if (!search || active === search || (active instanceof Element && active !== target && active.closest(KEEPS_FOCUS))) return;
        search.focus({ preventScroll: true });
      }, 0);
    };
    root.addEventListener("click", onClick);
    return () => {
      clearTimeout(timer);
      root.removeEventListener("click", onClick);
    };
  }, [pane, shown]);
}
