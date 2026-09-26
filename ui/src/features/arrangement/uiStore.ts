/**
 * Arrangement-local UI state: the timeline view (zoom/scroll, shared by the ruler and the
 * lanes), folded groups, the grid setting, the live drag preview and pending browser-drop
 * imports. None of it is in the
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

/** A browser drop still importing (placeholder in the lane), or one that failed. */
export interface PendingImport {
  id: string;
  /** Target track (`null` = a new track, shown in the drop area). */
  track: TrackId | null;
  at: Beats;
  name: string;
  /** Engine `ImportProgress` (0..1), if reported. */
  progress: number | null;
  /** Failure message (the placeholder stays a few seconds to show it). */
  error: string | null;
}

export interface ArrangementUiState {
  folded: ReadonlySet<TrackId>;
  grid: GridSetting;
  preview: DragPreview | null;
  /** Timeline position of a pending browser drop (indicator), with its track. */
  dropHint: { track: TrackId | null; at: Beats } | null;
  imports: ReadonlyArray<PendingImport>;

  toggleFold(track: TrackId): void;
  setGrid(grid: GridSetting): void;
  setPreview(preview: DragPreview | null): void;
  setDropHint(hint: ArrangementUiState["dropHint"]): void;
  /** Add or update (by id) a pending import. */
  putImport(item: PendingImport): void;
  removeImport(id: string): void;
}

const INITIAL = {
  folded: new Set<TrackId>() as ReadonlySet<TrackId>,
  grid: DEFAULT_GRID,
  preview: null,
  dropHint: null,
  imports: [] as ReadonlyArray<PendingImport>,
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
  putImport: (item) =>
    set((s) => ({
      imports: s.imports.some((i) => i.id === item.id) ? s.imports.map((i) => (i.id === item.id ? item : i)) : [...s.imports, item],
    })),
  removeImport: (id) => set((s) => ({ imports: s.imports.filter((i) => i.id !== id) })),
}));

const DEFAULT_PX_PER_BEAT = 24;

/** The arrangement's timeline view (ruler + lanes scroll and zoom together). */
export let arrangementView: TimelineViewStore = createTimelineViewStore({ pxPerBeat: DEFAULT_PX_PER_BEAT });

/** Reset all arrangement UI state (tests). */
export function resetArrangementUi(): void {
  useArrangementUi.setState({ ...INITIAL, folded: new Set() });
  arrangementView = createTimelineViewStore({ pxPerBeat: DEFAULT_PX_PER_BEAT });
}
