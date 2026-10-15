import { create } from "zustand";
import type { MessageKey } from "@/i18n";

/**
 * Whether the app shows the project screen on launch: not under test runners (vitest, or a
 * WebDriver/Playwright-driven browser), where it would cover the app. Tests that want it set
 * `launchPending` themselves.
 */
const launchByDefault = () => typeof navigator !== "undefined" && !navigator.webdriver && import.meta.env.MODE !== "test";

/** What the screen shows: the projects, the new-project form or the save-as form. */
export type ScreenMode = "home" | "new" | "saveAs";

interface ProjectScreenState {
  open: boolean;
  mode: ScreenMode;
  /** Bumped to rename the open project in place (its name in the top bar). */
  renameRequest: number;
  /** Show the screen once the first project is loaded (app launch). */
  launchPending: boolean;
  /**
   * Launch hasn't decided yet what to show (engine booting, reopening or making the draft):
   * the arranger stays blank instead of flashing its "No project open" state first.
   */
  launching: boolean;
  /**
   * Launch has shown the picker (early, before the engine is up, or once it decided): it
   * isn't shown again, e.g. after it was dismissed while the engine was loading.
   */
  launchShown: boolean;
  /** A line shown on the picker (why it is shown: the URL's project isn't stored here); cleared on hide. */
  notice: MessageKey | null;
  show(mode?: ScreenMode): void;
  /** Rename the open project in place (its name in the top bar becomes a field). */
  rename(): void;
  /** Open straight on the "New project" form (templates: "New project from template…"). */
  showNew(): void;
  hide(): void;
  setMode(mode: ScreenMode): void;
}

export const useProjectScreen = create<ProjectScreenState>((set) => ({
  open: false,
  mode: "home",
  renameRequest: 0,
  launchPending: launchByDefault(),
  launching: true,
  launchShown: false,
  notice: null,
  show: (mode = "home") => set({ open: true, mode }),
  rename: () => set((s) => ({ renameRequest: s.renameRequest + 1 })),
  showNew: () => set({ open: true, mode: "new" }),
  hide: () => set({ open: false, mode: "home", notice: null }),
  setMode: (mode) => set({ mode }),
}));
