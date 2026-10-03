import { create } from "zustand";

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
  /** Bumped to focus (and select) the open project's name field ("Rename"). */
  renameRequest: number;
  /** Show the screen once the first project is loaded (app launch). */
  launchPending: boolean;
  show(mode?: ScreenMode): void;
  /** Show the screen with the open project's name field focused. */
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
  show: (mode = "home") => set({ open: true, mode }),
  rename: () =>
    set((s) => ({
      open: true,
      mode: "home",
      renameRequest: s.renameRequest + 1,
    })),
  showNew: () => set({ open: true, mode: "new" }),
  hide: () => set({ open: false, mode: "home" }),
  setMode: (mode) => set({ mode }),
}));
