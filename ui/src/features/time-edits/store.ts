/**
 * The arrangement's time selection (a beat range over some tracks). UI state only: the time
 * clipboard lives in the engine (`TimeEdit::Copy`); `clipboard` only remembers its shape
 * (a refused edit, e.g. on a frozen track, is a warning toast).
 *
 * The range is mirrored into `itemSelection.timeRange` so other views (export "selection")
 * see it.
 *
 * A ZERO-LENGTH selection is the INSERT MARKER (Ableton's edit cursor, owner request): a plain
 * click on the lanes places it, and paste / split / ... target it instead of the playhead
 * (see `marker.ts`). It is view state like the range: never a document edit, never
 * replicated, and not mirrored as a time range.
 */

import { create } from "zustand";
import type { Beats, TrackId } from "@/generated";
import { useProjectStore } from "@/state/projectStore";
import { itemSelection } from "@/timeline";

export interface TimeRangeSelection {
  start: Beats;
  end: Beats;
  /** Selected tracks (audio, MIDI and group), in display order. */
  tracks: TrackId[];
  /**
   * A range drawn in ONE automation lane (its target key, on `tracks[0]`): the lane's own
   * selection (its range edits copy / cut / paste / duplicate / delete its automation), not
   * a section of the tracks: the track time edits ignore it.
   */
  automationLane?: string;
}

/** Below this length a selection is the insert marker. */
export const MARKER_EPS = 1e-6;

/** `true` if `sel` is the insert marker (a zero-length selection). */
export function isMarker(sel: TimeRangeSelection | null): sel is TimeRangeSelection {
  return sel !== null && sel.end - sel.start <= MARKER_EPS;
}

/** `sel` if it is a real (non-empty) time range, else null. */
export function rangeOf(sel: TimeRangeSelection | null): TimeRangeSelection | null {
  return sel && !isMarker(sel) ? sel : null;
}

/** `sel` if it is a real time range over whole tracks (not one automation lane's), else null. */
export function trackRangeOf(sel: TimeRangeSelection | null): TimeRangeSelection | null {
  const range = rangeOf(sel);
  return range && range.automationLane === undefined ? range : null;
}

export interface TimeSelectionState {
  selection: TimeRangeSelection | null;
  /** The last time copy/cut (the engine holds the material): its track count and length. */
  clipboard: { tracks: number; length: Beats } | null;
  /**
   * Song position the insert marker was placed at while playing: the next stop locates the
   * playhead there, so Play starts from the marker (clicks never move a playing playhead).
   */
  playFrom: Beats | null;
  /**
   * The SELECTION CURSOR: song position of the start of the
   * last selection made (arrangement time selection / insert marker, piano-roll section or
   * marker, a clip click). It outlives the selection itself (clicking a clip clears the time
   * selection, not the cursor). Edits without a range land here, never at the playhead;
   * Shift+Space and Stop return the playhead here. 0 until something is selected.
   */
  cursor: Beats;
  setSelection(selection: TimeRangeSelection | null): void;
  setClipboard(clipboard: TimeSelectionState["clipboard"]): void;
}

export const useTimeSelection = create<TimeSelectionState>()((set, get) => ({
  selection: null,
  clipboard: null,
  playFrom: null,
  cursor: 0,
  setSelection: (selection) => {
    if (selection === null && get().selection === null) return;
    set(selection ? { selection, cursor: Math.max(0, selection.start) } : { selection });
    const range = rangeOf(selection);
    itemSelection.getState().setTimeRange(range ? { start: range.start, end: range.end } : null);
  },
  setClipboard: (clipboard) => set({ clipboard }),
}));

/**
 * Keep the time selection on the open project: another project (or none) clears it,
 * the selection cursor and the play-from marker; deleted tracks leave it (a selection left
 * with no track goes). Returns the unsubscribe function.
 */
export function bindTimeSelectionToProject(): () => void {
  return useProjectStore.subscribe((s, prev) => {
    if (s.project === prev.project) return;
    const store = useTimeSelection.getState();
    if (s.project?.id !== prev.project?.id) {
      store.setSelection(null);
      useTimeSelection.setState({ cursor: 0, playFrom: null });
      return;
    }
    const sel = store.selection;
    if (!sel || !s.project) return;
    const tracks = s.project.tracks;
    const kept = sel.tracks.filter((t) => t in tracks);
    if (kept.length === sel.tracks.length) return;
    // A lane range lives on its track: it goes with it.
    store.setSelection(kept.length === 0 || (sel.automationLane && kept[0] !== sel.tracks[0]) ? null : { ...sel, tracks: kept });
  });
}
bindTimeSelectionToProject();

export function clearTimeSelection(): void {
  useTimeSelection.getState().setSelection(null);
}

/** Reset (tests). */
export function resetTimeSelection(): void {
  useTimeSelection.setState({ selection: null, clipboard: null, playFrom: null, cursor: 0 });
}
