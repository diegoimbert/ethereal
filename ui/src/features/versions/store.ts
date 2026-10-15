import { create } from "zustand";

interface VersionsDialogState {
  open: boolean;
  show(): void;
  hide(): void;
}

/** Whether the versions dialog is shown (opened from the project screen, menus, palette). */
export const useVersionsDialog = create<VersionsDialogState>((set) => ({
  open: false,
  show: () => set({ open: true }),
  hide: () => set({ open: false }),
}));

/** Open the versions dialog of the open project. */
export const openVersions = () => useVersionsDialog.getState().show();

/** The "save snapshot…" name prompt is shown (palette, `Mod+Alt+S`, project menu). */
export const useSnapshotPrompt = create<VersionsDialogState>((set) => ({
  open: false,
  show: () => set({ open: true }),
  hide: () => set({ open: false }),
}));

/** Ask for a name and save the open project as a named version (a snapshot). */
export const openSnapshotPrompt = () => useSnapshotPrompt.getState().show();

/** base-131: the crash-recovery dialog is up (the project screen waits behind it). */
export const useRecoveryShown = create<{ shown: boolean }>(() => ({ shown: false }));
