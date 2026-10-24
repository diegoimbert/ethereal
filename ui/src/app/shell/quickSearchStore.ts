import { create } from "zustand";

interface QuickSearchState {
  open: boolean;
  setOpen(open: boolean): void;
}

/** Whether the quick search (⌘F, `QuickSearch.tsx`) is showing. */
export const useQuickSearch = create<QuickSearchState>((set) => ({
  open: false,
  setOpen: (open) => set({ open }),
}));
