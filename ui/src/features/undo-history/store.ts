// History panel UI state (local): the step whose checkpoint name is being edited, and a
// request from elsewhere (command palette) to name the current step.
import { create } from "zustand";

export interface UndoHistoryUiState {
  /** Step whose checkpoint name is being edited. */
  editing: number | null;
  /** Edit the current step's name once the panel has the list (palette "Name checkpoint…"). */
  editCurrent: boolean;
  setEditing(step: number | null): void;
  requestEditCurrent(): void;
}

export const useUndoHistoryUi = create<UndoHistoryUiState>()((set) => ({
  editing: null,
  editCurrent: false,
  setEditing: (editing) => set({ editing, editCurrent: false }),
  requestEditCurrent: () => set({ editCurrent: true }),
}));
