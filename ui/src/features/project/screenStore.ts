import { create } from "zustand";

/**
 * Whether the app shows the project screen on launch: not under test runners (vitest, or a
 * WebDriver/Playwright-driven browser), where it would cover the app. Tests that want it set
 * `launchPending` themselves.
 */
const launchByDefault = () => typeof navigator !== "undefined" && !navigator.webdriver && import.meta.env.MODE !== "test";

interface ProjectScreenState {
  open: boolean;
  /** Show the screen once the first project is loaded (app launch). */
  launchPending: boolean;
  /** The "New project" form is showing (instead of the project list). */
  naming: boolean;
  show(): void;
  /** Open straight on the "New project" form (templates: "New project from template…"). */
  showNew(): void;
  hide(): void;
  setNaming(naming: boolean): void;
}

export const useProjectScreen = create<ProjectScreenState>((set) => ({
  open: false,
  launchPending: launchByDefault(),
  naming: false,
  show: () => set({ open: true }),
  showNew: () => set({ open: true, naming: true }),
  hide: () => set({ open: false, naming: false }),
  setNaming: (naming) => set({ naming }),
}));
