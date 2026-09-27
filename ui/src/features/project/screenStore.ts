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
  show(): void;
  hide(): void;
}

export const useProjectScreen = create<ProjectScreenState>((set) => ({
  open: false,
  launchPending: launchByDefault(),
  show: () => set({ open: true }),
  hide: () => set({ open: false }),
}));
