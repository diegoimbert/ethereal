// The browser tab's title (and the desktop window's): `ethereal - <what's shown>` (lowercase). No
// store or engine imports: the docs page and the join landing use it before the app loads.
import { useEffect } from "react";

/** The brand, lowercase in the title like the rest of the UI. */
export const BRAND = "ethereal";
/** Marks unsaved changes, in front of the title. */
export const DIRTY_MARK = "• ";

/**
 * The title for `part` (the project's name, the docs page…): `ethereal - part`, or plain
 * `ethereal` without one. User text (a project name) is used as is, never translated.
 */
export function tabTitle(part?: string | null, dirty = false): string {
  const trimmed = part?.trim();
  return `${dirty ? DIRTY_MARK : ""}${trimmed ? `${BRAND} - ${trimmed}` : BRAND}`;
}

const inTauri = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/** Show `title` in the tab, and in the desktop window's title bar. */
export function setTabTitle(title: string): void {
  if (document.title === title) return;
  document.title = title;
  if (inTauri()) void import("@tauri-apps/api/window").then(({ getCurrentWindow }) => getCurrentWindow().setTitle(title)).catch(() => undefined);
}

/** Keep the title at `tabTitle(part, dirty)` while mounted; back to plain `ethereal` after. */
export function useTabTitle(part: string | null | undefined, dirty = false): void {
  const title = tabTitle(part, dirty);
  useEffect(() => setTabTitle(title), [title]);
  useEffect(() => () => setTabTitle(BRAND), []);
}
