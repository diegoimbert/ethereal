import { create } from "zustand";

/**
 * App-wide right-click menu state. `<ContextMenuHost />` (mounted once by the app shell)
 * swallows every native context menu except in text fields; a component that has actions
 * calls `openContextMenu(e, items)` from its `onContextMenu`. Right-clicking anything else
 * does nothing.
 */

export interface MenuItem {
  label: string;
  /** Shortcut hint shown on the right (display only). */
  shortcut?: string;
  danger?: boolean;
  disabled?: boolean;
  onSelect: () => void;
}

export type MenuEntry = MenuItem | "separator";

export interface OpenMenu {
  /** Bumped on every open, so the menu remounts with fresh state. */
  id: number;
  x: number;
  y: number;
  items: ReadonlyArray<MenuEntry>;
}

interface MenuState {
  menu: OpenMenu | null;
  close(): void;
}

export const useContextMenuStore = create<MenuState>()((set) => ({
  menu: null,
  close: () => set({ menu: null }),
}));

let nextId = 1;

/** Open the menu at the pointer. With no items it only suppresses the native menu. */
export function openContextMenu(
  e: { clientX: number; clientY: number; preventDefault(): void; stopPropagation(): void },
  items: ReadonlyArray<MenuEntry>,
): void {
  e.preventDefault();
  e.stopPropagation();
  if (!items.some((i) => i !== "separator")) return;
  useContextMenuStore.setState({ menu: { id: nextId++, x: e.clientX, y: e.clientY, items } });
}

/** Platform modifier label for shortcut hints ("⌘" on macOS, "Ctrl+" elsewhere). */
export const MOD_KEY = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform) ? "⌘" : "Ctrl+";
