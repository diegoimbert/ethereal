/**
 * Arrangement-local UI state: the timeline view (zoom/scroll, shared by the ruler and the
 * lanes), folded groups, lane heights, the grid setting, the live drag preview and pending browser-drop
 * imports. None of it is in the
 * document. Module-level so zoom and folding survive the view unmounting and remounting.
 */

import { create } from "zustand";
import type { Beats, ClipId, TrackId } from "@/generated";
import { createTimelineViewStore, DEFAULT_GRID, type GridSetting, type TimelineViewStore } from "@/timeline";
import type { ClipBounds } from "./editMath";
import type { NewLane } from "./newTrackDrag";
import { clampHeaderWidth, clampTrackHeight, HEADER_WIDTH, TRACK_HEIGHT, type DraftTrack } from "./layout";

export interface DragPreview {
  /** New bounds per dragged clip. */
  bounds: ReadonlyMap<ClipId, ClipBounds>;
  /** Copy drag: the originals stay and ghosts are drawn at `bounds`. */
  copy: boolean;
  /**
   * Dragged below the last track: the tracks the drop creates (ghost lanes under the last
   * track; `bounds` targets their ids). See newTrackDrag.ts.
   */
  newTracks?: ReadonlyArray<NewLane>;
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
  /**
   * The track selected as an entity (its header was clicked): Delete deletes it. The
   * arrangement has one selected entity at a time, so this is null while clips or
   * automation points are selected (see `bindSingleSelection`).
   */
  /** Width of the track header column (px), resizable; remembered across sessions. */
  headerWidth: number;
  trackFocus: TrackId | null;
  /** Tracks selected as entities (cmd-click toggles, shift-click a range); includes `trackFocus`. */
  selectedTracks: ReadonlySet<TrackId>;
  /** A track header being dragged: the drop indicator (a line at `y`, or a group to go into). */
  trackDrag: { track: TrackId; y: number | null; into: TrackId | null } | null;
  /** A track being added: its row asks for the type in place (see layout.ts `DraftTrack`). */
  draftTrack: DraftTrack | null;
  folded: ReadonlySet<TrackId>;
  /** Lane height of resized tracks; the others use `defaultHeight`. */
  heights: ReadonlyMap<TrackId, number>;
  /** Lane height of tracks never resized individually (scaled with the others). */
  defaultHeight: number;
  grid: GridSetting;
  preview: DragPreview | null;
  /** Timeline position of a pending browser drop (indicator), with its track. */
  dropHint: { track: TrackId | null; at: Beats } | null;
  imports: ReadonlyArray<PendingImport>;

  setHeaderWidth(px: number): void;
  /** Select just `track` (or nothing). */
  setTrackFocus(track: TrackId | null): void;
  /** Select several tracks; `focus` is the one the inspector shows (the anchor of a range). */
  setTrackSelection(tracks: Iterable<TrackId>, focus: TrackId | null): void;
  setTrackDrag(drag: ArrangementUiState["trackDrag"]): void;
  setDraftTrack(draft: DraftTrack | null): void;
  toggleFold(track: TrackId): void;
  /** Resize one lane (clamped); `null` resets it to the default. */
  setHeight(track: TrackId, height: number | null): void;
  /** Multiply every lane height (and the default) by `factor`, each clamped. */
  scaleHeights(factor: number): void;
  setGrid(grid: GridSetting): void;
  setPreview(preview: DragPreview | null): void;
  setDropHint(hint: ArrangementUiState["dropHint"]): void;
  /** Add or update (by id) a pending import. */
  putImport(item: PendingImport): void;
  removeImport(id: string): void;
}

const HEADER_WIDTH_KEY = "eth.arr.headerWidth";

function savedHeaderWidth(): number {
  try {
    const v = Number(localStorage.getItem(HEADER_WIDTH_KEY));
    return Number.isFinite(v) && v > 0 ? clampHeaderWidth(v) : HEADER_WIDTH;
  } catch {
    return HEADER_WIDTH;
  }
}

const INITIAL = {
  trackFocus: null as TrackId | null,
  selectedTracks: new Set<TrackId>() as ReadonlySet<TrackId>,
  trackDrag: null as ArrangementUiState["trackDrag"],
  draftTrack: null as DraftTrack | null,
  folded: new Set<TrackId>() as ReadonlySet<TrackId>,
  heights: new Map<TrackId, number>() as ReadonlyMap<TrackId, number>,
  defaultHeight: TRACK_HEIGHT,
  grid: DEFAULT_GRID,
  preview: null,
  dropHint: null,
  imports: [] as ReadonlyArray<PendingImport>,
};

export const useArrangementUi = create<ArrangementUiState>()((set) => ({
  ...INITIAL,
  headerWidth: savedHeaderWidth(),
  setHeaderWidth: (px) => {
    const headerWidth = clampHeaderWidth(px);
    set({ headerWidth });
    try {
      localStorage.setItem(HEADER_WIDTH_KEY, String(Math.round(headerWidth)));
    } catch {
      /* not persisted */
    }
  },
  setTrackFocus: (trackFocus) => set({ trackFocus, selectedTracks: new Set(trackFocus ? [trackFocus] : []) }),
  setTrackSelection: (tracks, focus) => set({ trackFocus: focus, selectedTracks: new Set(tracks) }),
  setTrackDrag: (trackDrag) => set({ trackDrag }),
  setDraftTrack: (draftTrack) => set({ draftTrack }),
  toggleFold: (track) =>
    set((s) => {
      const folded = new Set(s.folded);
      if (folded.has(track)) folded.delete(track);
      else folded.add(track);
      return { folded };
    }),
  setHeight: (track, height) =>
    set((s) => {
      const heights = new Map(s.heights);
      if (height === null) heights.delete(track);
      else heights.set(track, clampTrackHeight(height));
      return { heights };
    }),
  scaleHeights: (factor) =>
    set((s) => ({
      defaultHeight: clampTrackHeight(s.defaultHeight * factor),
      heights: new Map([...s.heights].map(([id, h]) => [id, clampTrackHeight(h * factor)])),
    })),
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
  useArrangementUi.setState({ ...INITIAL, folded: new Set(), heights: new Map(), selectedTracks: new Set(), headerWidth: HEADER_WIDTH });
  arrangementView = createTimelineViewStore({ pxPerBeat: DEFAULT_PX_PER_BEAT });
}
