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

/**
 * `chat`: collab-social's Chat section, shown only in a collaboration session.
 * `ai`: the AI chat (`ai-chat`).
 */
export type LeftTab = "library" | "project" | "plugins" | "devices" | "midi" | "history" | "chat" | "ai";
export type DrawerTab = "piano-roll" | "warp" | "automation" | "tempo" | "groove" | "drum-rack";
export type PaneSide = "left" | "right" | "bottom";

export interface PaneState {
  open: boolean;
  pinned: boolean;
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
    left: { open: false, pinned: false, size: px(size.browserWidth), tab: "library" as LeftTab, ...saved.left },
    // The inspector opens with the selection (never restored open).
    right: { pinned: false, size: px(size.inspectorWidth), ...saved.right, open: false },
    bottom: { open: false, pinned: false, size: px(size.drawerHeight), tab: "piano-roll" as DrawerTab, ...saved.bottom },
    paletteOpen: false,
  };
}

export const useShellStore = create<ShellState>()((set, get) => {
  const save = () => {
    const { left, right, bottom } = get();
    try {
      localStorage.setItem(KEY, JSON.stringify({ left, right: { pinned: right.pinned, size: right.size }, bottom }));
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
