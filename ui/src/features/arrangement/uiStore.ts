/**
 * Arrangement-local UI state: the timeline view (zoom/scroll, shared by the ruler and the
 * lanes), folded groups, lane heights, the grid setting, the live drag preview and pending browser-drop
 * imports. None of it is in the
 * document. Module-level so zoom and folding survive the view unmounting and remounting.
 */

import { create } from "zustand";
import type { Beats, ClipId, TrackId } from "@/generated";
import { useProjectStore } from "@/state/projectStore";
import { createTimelineViewStore, DEFAULT_GRID, type GridSetting, type TimelineViewStore } from "@/timeline";
import type { ClipBounds } from "./editMath";
import type { NewLane } from "./newTrackDrag";
import { clampHeaderWidth, clampTrackHeight, HEADER_WIDTH, TRACK_HEIGHT } from "./layout";

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
  /**
   * A track header being dragged: the drop indicator (a line at `y`, or a group to go into).
   * `blocked`: a return over the regular tracks, the zone (content px) it can't be dropped in.
   */
  trackDrag: { track: TrackId; y: number | null; into: TrackId | null; blocked?: { top: number; height: number } } | null;
  /** A MIDI track just dragged into place from Add track: its header offers the instruments. */
  instrumentPick: TrackId | null;
  /** A group whose header is hovered: its tracks (at any depth) look hovered too, not selected. */
  hoveredGroup: TrackId | null;
  folded: ReadonlySet<TrackId>;
  /** Lane height of resized tracks; the others use `defaultHeight`. */
  heights: ReadonlyMap<TrackId, number>;
  /** Lane height of tracks never resized individually (scaled with the others). */
  defaultHeight: number;
  grid: GridSetting;
  preview: DragPreview | null;
  /** Timeline position of a pending browser drop (indicator), with its track. */
  dropHint: { track: TrackId | null; at: Beats } | null;
  /** The track device cards are dragged over (it takes them on drop: `@/features/devices/trackDrop`). */
  deviceDrop: TrackId | null;
  imports: ReadonlyArray<PendingImport>;

  setHeaderWidth(px: number): void;
  /** Select just `track` (or nothing). */
  setTrackFocus(track: TrackId | null): void;
  /** Select several tracks; `focus` is the one the inspector shows (the anchor of a range). */
  setTrackSelection(tracks: Iterable<TrackId>, focus: TrackId | null): void;
  setTrackDrag(drag: ArrangementUiState["trackDrag"]): void;
  setInstrumentPick(track: TrackId | null): void;
  setHoveredGroup(group: TrackId | null): void;
  toggleFold(track: TrackId): void;
  /** Resize one lane (clamped); `null` resets it to the default. */
  setHeight(track: TrackId, height: number | null): void;
  /** Multiply every lane height (and the default) by `factor`, each clamped. */
  scaleHeights(factor: number): void;
  setGrid(grid: GridSetting): void;
  setPreview(preview: DragPreview | null): void;
  setDropHint(hint: ArrangementUiState["dropHint"]): void;
  setDeviceDrop(track: TrackId | null): void;
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
  instrumentPick: null as TrackId | null,
  hoveredGroup: null as TrackId | null,
  folded: new Set<TrackId>() as ReadonlySet<TrackId>,
  heights: new Map<TrackId, number>() as ReadonlyMap<TrackId, number>,
  defaultHeight: TRACK_HEIGHT,
  grid: DEFAULT_GRID,
  preview: null,
  dropHint: null,
  deviceDrop: null,
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
  setInstrumentPick: (instrumentPick) => set({ instrumentPick }),
  setHoveredGroup: (hoveredGroup) => set((s) => (s.hoveredGroup === hoveredGroup ? s : { hoveredGroup })),
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
  setDeviceDrop: (deviceDrop) => set((s) => (s.deviceDrop === deviceDrop ? s : { deviceDrop })),
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

/**
 * Keep the selected tracks on the open project: another project clears them (the
 * inspector closes), deleted tracks drop out (an undone "add track" closes its inspector).
 * Returns the unsubscribe function.
 */
export function bindTrackSelectionToProject(): () => void {
  return useProjectStore.subscribe((s, prev) => {
    if (s.project === prev.project) return;
    const { trackFocus, selectedTracks } = useArrangementUi.getState();
    if (trackFocus === null && selectedTracks.size === 0) return;
    if (s.project?.id !== prev.project?.id || !s.project) {
      useArrangementUi.setState({ trackFocus: null, selectedTracks: new Set() });
      return;
    }
    const tracks = s.project.tracks;
    const kept = [...selectedTracks].filter((t) => t in tracks);
    const focus = trackFocus !== null && trackFocus in tracks ? trackFocus : null;
    if (focus === trackFocus && kept.length === selectedTracks.size) return;
    useArrangementUi.setState({ trackFocus: focus, selectedTracks: new Set(kept) });
  });
}
bindTrackSelectionToProject();
