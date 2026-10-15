import { create } from "zustand";

/** Whether the Export audio dialog is open (opened from the project menu). */
export const useExportAudio = create<{ open: boolean; show(): void; hide(): void }>((set) => ({
  open: false,
  show: () => set({ open: true }),
  hide: () => set({ open: false }),
}));
