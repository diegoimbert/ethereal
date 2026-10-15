import { create } from "zustand";
import { size } from "@/theme";

/**
 * Layout state of the floating panes, remembered across sessions (localStorage):
 *
 * - `left`: the browser panel opened from the icon rail (`tab` = which panel);
 * - `right`: the inspector (open whenever something is selected, see `Inspector`);
 * - `bottom`: the editor drawer (piano roll, warp, automation, ...).
 *
 * Unpinned panes float over the arrangement; pinned ones also reserve their space (the
 * arrangement shrinks), still drawn as floating cards.
 */

/** `ai`: the AI chat (`ai-chat`). */
export type LeftTab = "library" | "midi" | "history" | "ai";
export type DrawerTab = "piano-roll" | "warp" | "automation";
const DRAWER_TAB_IDS: ReadonlyArray<DrawerTab> = ["piano-roll", "warp", "automation"];
export type PaneSide = "left" | "right" | "bottom";

export interface PaneState {
  open: boolean;
  pinned: boolean;
  /**
   * Right/bottom only: shown as a thin floating bar instead of the card (the panel's × button
   * collapses; the bar expands it). A remembered preference: a panel that opens again keeps it.
   */
  collapsed: boolean;
  /** Width (left/right) or height (bottom), px. */
  size: number;
}

export interface ShellState {
  left: PaneState & { tab: LeftTab };
  right: PaneState;
  bottom: PaneState & { tab: DrawerTab };
  paletteOpen: boolean;

  /** Rail click: open `tab`, or close the pane when it is already showing it. */
  toggleLeft(tab: LeftTab): void;
  openDrawer(tab: DrawerTab): void;
  setOpen(side: PaneSide, open: boolean): void;
  setPinned(side: PaneSide, pinned: boolean): void;
  setCollapsed(side: PaneSide, collapsed: boolean): void;
  /**
   * Expand a collapsed right / bottom pane at its last size, clamped to `[min, max]` (the room
   * left once the arrangement keeps its minimum size). A pane already expanded is left as is.
   */
  expand(side: "right" | "bottom", min: number, max: number): void;
  setSize(side: PaneSide, px: number): void;
  setPalette(open: boolean): void;
}

const px = (token: string) => parseFloat(token);
const KEY = "eth.shell";

interface Saved {
  left?: Partial<PaneState> & { tab?: LeftTab };
  right?: Partial<PaneState>;
  bottom?: Partial<PaneState> & { tab?: DrawerTab };
}

const LEFT_TABS: ReadonlyArray<LeftTab> = ["library", "midi", "history", "ai"];
const leftTab = (tab: string | undefined): LeftTab => LEFT_TABS.find((t) => t === tab) ?? "library";

function load(): Saved {
  try {
    return JSON.parse(localStorage.getItem(KEY) ?? "{}") as Saved;
  } catch {
    return {};
  }
}

function initial() {
  const saved = load();
  return {
    // A removed tab (the former "project" media tab) falls back to the library.
    left: { open: false, pinned: false, collapsed: false, size: px(size.browserWidth), ...saved.left, tab: leftTab(saved.left?.tab) },
    // The inspector opens with the selection (never restored open).
    right: { pinned: false, collapsed: false, size: px(size.inspectorWidth), ...saved.right, open: false },
    bottom: {
      open: false,
      pinned: false,
      collapsed: false,
      size: px(size.drawerHeight),
      ...saved.bottom,
      // A tab that no longer exists (Tempo, Groove, Drum Rack) opens the piano roll.
      tab: saved.bottom?.tab && DRAWER_TAB_IDS.includes(saved.bottom.tab) ? saved.bottom.tab : ("piano-roll" as DrawerTab),
    },
    paletteOpen: false,
  };
}

export const useShellStore = create<ShellState>()((set, get) => {
  const save = () => {
    const { left, right, bottom } = get();
    try {
      localStorage.setItem(KEY, JSON.stringify({ left, right: { pinned: right.pinned, size: right.size, collapsed: right.collapsed }, bottom }));
    } catch {
      /* not persisted */
    }
  };
  const patch = (side: PaneSide, p: Partial<PaneState>) => {
    set((s) => ({ [side]: { ...s[side], ...p } }) as Partial<ShellState>);
    save();
  };
  return {
    ...initial(),
    toggleLeft: (tab) => {
      const { left } = get();
      patch("left", left.open && left.tab === tab ? { open: false } : ({ open: true, tab } as Partial<PaneState>));
    },
    openDrawer: (tab) => patch("bottom", { open: true, tab } as Partial<PaneState>),
    setOpen: (side, open) => patch(side, { open }),
    setPinned: (side, pinned) => patch(side, { pinned }),
    setCollapsed: (side, collapsed) => patch(side, { collapsed }),
    expand: (side, min, max) => {
      const pane = get()[side];
      if (pane.collapsed) patch(side, { collapsed: false, size: Math.max(min, Math.min(max, pane.size)) });
    },
    setSize: (side, size) => patch(side, { size }),
    setPalette: (paletteOpen) => set({ paletteOpen }),
  };
});

/** Tests: back to defaults (ignores what was saved). */
export function resetShell(): void {
  try {
    localStorage.removeItem(KEY);
  } catch {
    /* nothing saved */
  }
  useShellStore.setState(initial());
}

/** Whether the left pane shows `tab`. */
export function leftTabOpen(tab: LeftTab): boolean {
  const { left } = useShellStore.getState();
  return left.open && left.tab === tab;
}

/** Show `tab` in the left pane (no-op when it already does). */
export function openLeftTab(tab: LeftTab): void {
  if (!leftTabOpen(tab)) useShellStore.getState().toggleLeft(tab);
}

/** Hook: whether the left pane shows `tab`. */
export const useLeftTabOpen = (tab: LeftTab): boolean => useShellStore((s) => s.left.open && s.left.tab === tab);
