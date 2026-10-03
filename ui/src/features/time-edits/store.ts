/**
 * The arrangement's time selection (a beat range over some tracks) and the time-edit
 * notice (the last refused edit, e.g. a frozen track). UI state only: the time clipboard
 * lives in the engine (`TimeEdit::Copy`); `clipboard` only remembers its shape.
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
import { itemSelection } from "@/timeline";

export interface TimeRangeSelection {
  start: Beats;
  end: Beats;
  /** Selected tracks (audio, MIDI and group), in display order. */
  tracks: TrackId[];
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

export interface TimeEditNotice {
  id: number;
  message: string;
}

export interface TimeSelectionState {
  selection: TimeRangeSelection | null;
  /** The last time copy/cut (the engine holds the material): its track count and length. */
  clipboard: { tracks: number; length: Beats } | null;
  notice: TimeEditNotice | null;
  /**
   * Song position the insert marker was placed at while playing: the next stop locates the
   * playhead there, so Play starts from the marker (clicks never move a playing playhead).
   */
  playFrom: Beats | null;
  setSelection(selection: TimeRangeSelection | null): void;
  setClipboard(clipboard: TimeSelectionState["clipboard"]): void;
  notify(message: string | null): void;
}

let nextNotice = 1;

export const useTimeSelection = create<TimeSelectionState>()((set, get) => ({
  selection: null,
  clipboard: null,
  notice: null,
  playFrom: null,
  setSelection: (selection) => {
    if (selection === null && get().selection === null) return;
    set({ selection });
    const range = rangeOf(selection);
    itemSelection.getState().setTimeRange(range ? { start: range.start, end: range.end } : null);
  },
  setClipboard: (clipboard) => set({ clipboard }),
  notify: (message) => set({ notice: message === null ? null : { id: nextNotice++, message } }),
}));

export function clearTimeSelection(): void {
  useTimeSelection.getState().setSelection(null);
}

/** Reset (tests). */
export function resetTimeSelection(): void {
  useTimeSelection.setState({ selection: null, clipboard: null, notice: null, playFrom: null });
}
