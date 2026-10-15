/**
 * base-154: debug panels. A feature registers a panel once (`registerDebugPanel`); the app
 * shows the open ones in kit `FloatingWindow`s (`DebugPanelsRoot`), lists each in the command
 * palette ("Debug: <title>") and in Settings › Advanced. Which panels are open is remembered on
 * this machine (localStorage `eth.debug.panels`); each window's geometry is the kit's, under
 * the window id `debug:<id>`. See README.md.
 */
import type { ReactNode } from "react";
import { create } from "zustand";
import { useFloatingWindows } from "@/kit";
import { t } from "@/i18n";

export interface DebugPanelSize {
  width: number;
  height: number;
}

export interface DebugPanel {
  /** Stable id (the open state and the window geometry are remembered under it). */
  id: string;
  /** Shown in the window's title bar, the palette ("Debug: <title>") and the settings. */
  title: string;
  /**
   * The panel's content. It fills a resizable window: lay it out as a flex column filling the
   * height, with its own scrolling regions (`min-height: 0; overflow: auto`), never wider than
   * the window.
   */
  render: () => ReactNode;
  /** The window's size before the user resizes it (default: the kit's). */
  defaultSize?: DebugPanelSize;
  /** Extra palette search words. */
  keywords?: string;
  /** Called when the panel opens (e.g. to turn on what feeds it). */
  onOpen?: () => void;
}

const KEY = "eth.debug.panels";

interface DebugPanelsState {
  /** Registered panels, in registration order. */
  panels: ReadonlyArray<DebugPanel>;
  /** Open panel ids (registered or not yet). */
  open: ReadonlyArray<string>;
}

function load(): string[] {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "[]") as unknown;
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : [];
  } catch {
    return [];
  }
}

export const useDebugPanels = create<DebugPanelsState>(() => ({ panels: [], open: load() }));

useDebugPanels.subscribe((s, prev) => {
  if (s.open === prev.open) return;
  try {
    localStorage.setItem(KEY, JSON.stringify(s.open));
  } catch {
    /* not persisted (storage unavailable) */
  }
});

/** Registers (or replaces) a debug panel; returns its unregistration. */
export function registerDebugPanel(panel: DebugPanel): () => void {
  useDebugPanels.setState((s) => {
    const i = s.panels.findIndex((p) => p.id === panel.id);
    const panels = [...s.panels];
    if (i >= 0) panels[i] = panel;
    else panels.push(panel);
    return { panels };
  });
  return () => useDebugPanels.setState((s) => ({ panels: s.panels.filter((p) => p !== panel) }));
}

export function debugPanel(id: string): DebugPanel | undefined {
  return useDebugPanels.getState().panels.find((p) => p.id === id);
}

export function setDebugPanelOpen(id: string, open: boolean): void {
  const s = useDebugPanels.getState();
  if (s.open.includes(id) === open) {
    // Opening an open panel brings it up.
    if (open) {
      useFloatingWindows.getState().raise(debugWindowId(id));
      [...document.querySelectorAll<HTMLElement>("[data-debug-panel]")].find((el) => el.dataset.debugPanel === id)?.focus();
    }
    return;
  }
  if (open) debugPanel(id)?.onOpen?.();
  useDebugPanels.setState({ open: open ? [...s.open, id] : s.open.filter((x) => x !== id) });
}

export const openDebugPanel = (id: string) => setDebugPanelOpen(id, true);
export const closeDebugPanel = (id: string) => setDebugPanelOpen(id, false);

export function useDebugPanelOpen(id: string): boolean {
  return useDebugPanels((s) => s.open.includes(id));
}

/** The kit `FloatingWindow` id of a debug panel's window. */
export function debugWindowId(id: string): string {
  return `debug:${id}`;
}

export interface DebugPaletteCommand {
  id: string;
  label: string;
  group: string;
  keywords?: string;
  run(): void;
}

/** One palette entry per registered panel: "Debug: <title>" (opens it, or brings it up). */
export function debugPanelCommands(): DebugPaletteCommand[] {
  const { panels } = useDebugPanels.getState();
  return panels.map((p) => ({
    id: `debug:${p.id}`,
    label: t("debug.command", { title: p.title }),
    group: "Debug",
    keywords: `debug panel ${p.keywords ?? ""}`.trim(),
    run: () => openDebugPanel(p.id),
  }));
}

/** Tests: no panel open (storage cleared too); registrations stay. */
export function resetDebugPanels(): void {
  useDebugPanels.setState({ open: [] });
  try {
    localStorage.removeItem(KEY);
  } catch {
    /* ignore */
  }
}
