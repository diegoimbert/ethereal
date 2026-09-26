/**
 * Arrangement-local UI state: the timeline view (zoom/scroll, shared by the ruler and the
 * lanes), folded groups, the grid setting and the live drag preview. None of it is in the
 * document. Module-level so zoom and folding survive the view unmounting and remounting.
 */

import { create } from "zustand";
import type { Beats, ClipId, TrackId } from "@/generated";
import { createTimelineViewStore, DEFAULT_GRID, type GridSetting, type TimelineViewStore } from "@/timeline";
import type { ClipBounds } from "./editMath";

export interface DragPreview {
  /** New bounds per dragged clip. */
  bounds: ReadonlyMap<ClipId, ClipBounds>;
  /** Copy drag: the originals stay and ghosts are drawn at `bounds`. */
  copy: boolean;
}

export interface ArrangementUiState {
  folded: ReadonlySet<TrackId>;
  grid: GridSetting;
  preview: DragPreview | null;
  /** Timeline position of a pending browser drop (indicator), with its track. */
  dropHint: { track: TrackId | null; at: Beats } | null;

  toggleFold(track: TrackId): void;
  setGrid(grid: GridSetting): void;
  setPreview(preview: DragPreview | null): void;
  setDropHint(hint: ArrangementUiState["dropHint"]): void;
}

const INITIAL = {
  folded: new Set<TrackId>() as ReadonlySet<TrackId>,
  grid: DEFAULT_GRID,
  preview: null,
  dropHint: null,
};

export const useArrangementUi = create<ArrangementUiState>()((set) => ({
  ...INITIAL,
  toggleFold: (track) =>
    set((s) => {
      const folded = new Set(s.folded);
      if (folded.has(track)) folded.delete(track);
      else folded.add(track);
      return { folded };
    }),
  setGrid: (grid) => set({ grid }),
  setPreview: (preview) => set({ preview }),
  setDropHint: (dropHint) => set({ dropHint }),
}));

const DEFAULT_PX_PER_BEAT = 24;

/** The arrangement's timeline view (ruler + lanes scroll and zoom together). */
export let arrangementView: TimelineViewStore = createTimelineViewStore({ pxPerBeat: DEFAULT_PX_PER_BEAT });

/** Reset all arrangement UI state (tests). */
export function resetArrangementUi(): void {
  useArrangementUi.setState({ ...INITIAL, folded: new Set() });
  arrangementView = createTimelineViewStore({ pxPerBeat: DEFAULT_PX_PER_BEAT });
}
