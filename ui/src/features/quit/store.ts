// Whether the "Quit Ethereal?" confirmation is showing (desktop).
import { create } from "zustand";

interface QuitConfirmState {
  open: boolean;
  setOpen(open: boolean): void;
}

export const useQuitConfirm = create<QuitConfirmState>((set) => ({
  open: false,
  setOpen: (open) => set({ open }),
}));
