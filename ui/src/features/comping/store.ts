/**
 * Take-lane UI state (not in the document): which tracks show their take lanes, the
 * selected comp region (for the next/previous take shortcuts), the swipe being dragged,
 * and the auditioned lane per track (mirrors the runtime `Take::Audition` this client sent).
 * A refused edit (e.g. a frozen track) is a warning toast.
 */

import { create } from "zustand";
import type { Beats, CompRegionId, TakeLaneId, TrackId } from "@/generated";

export interface SwipePreview {
  track: TrackId;
  lane: TakeLaneId;
  start: Beats;
  end: Beats;
}

export interface CompingUiState {
  expanded: ReadonlySet<TrackId>;
  /** The selected comp region (Up/Down pick the previous/next take for it). */
  selected: { track: TrackId; region: CompRegionId } | null;
  swipe: SwipePreview | null;
  audition: ReadonlyMap<TrackId, TakeLaneId>;
  toggle(track: TrackId, open?: boolean): void;
  select(sel: { track: TrackId; region: CompRegionId } | null): void;
  setSwipe(swipe: SwipePreview | null): void;
  setAudition(track: TrackId, lane: TakeLaneId | null): void;
}

export const useCompingUi = create<CompingUiState>((set) => ({
  expanded: new Set(),
  selected: null,
  swipe: null,
  audition: new Map(),
  toggle: (track, open) =>
    set((s) => {
      const next = new Set(s.expanded);
      const want = open ?? !next.has(track);
      if (want) next.add(track);
      else next.delete(track);
      return { expanded: next };
    }),
  select: (selected) => set({ selected }),
  setSwipe: (swipe) => set({ swipe }),
  setAudition: (track, lane) =>
    set((s) => {
      const next = new Map(s.audition);
      if (lane) next.set(track, lane);
      else next.delete(track);
      return { audition: next };
    }),
}));
