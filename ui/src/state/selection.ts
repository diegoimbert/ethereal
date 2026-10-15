// Shared UI selection that crosses features: the "selected track" (Ableton: clicking a track
// anywhere shows its devices). Per-item selection inside a view (clips, notes, automation
// points) lives in `ui/src/timeline`. Consumers must tolerate an id that no longer exists
// (e.g. after ProjectLoaded) and treat it as no selection.
import { create } from "zustand";
import type { TrackId } from "@/generated";

export interface SelectionState {
  selectedTrack: TrackId | null;
  selectTrack(id: TrackId | null): void;
}

export const useSelectionStore = create<SelectionState>()((set) => ({
  selectedTrack: null,
  selectTrack: (id) => set({ selectedTrack: id }),
}));

export const useSelectedTrackId = (): TrackId | null => useSelectionStore((s) => s.selectedTrack);
