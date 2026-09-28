/**
 * The arrangement's time selection (a beat range over some tracks) and the time-edit
 * notice (the last refused edit, e.g. a frozen track). UI state only: the time clipboard
 * lives in the engine (`TimeEdit::Copy`); `clipboard` only remembers its shape.
 *
 * The range is mirrored into `itemSelection.timeRange` so other views (export "selection")
 * see it.
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

export interface TimeEditNotice {
  id: number;
  message: string;
}

export interface TimeSelectionState {
  selection: TimeRangeSelection | null;
  /** The last time copy/cut (the engine holds the material): its track count and length. */
  clipboard: { tracks: number; length: Beats } | null;
  notice: TimeEditNotice | null;
  setSelection(selection: TimeRangeSelection | null): void;
  setClipboard(clipboard: TimeSelectionState["clipboard"]): void;
  notify(message: string | null): void;
}

let nextNotice = 1;

export const useTimeSelection = create<TimeSelectionState>()((set, get) => ({
  selection: null,
  clipboard: null,
  notice: null,
  setSelection: (selection) => {
    if (selection === null && get().selection === null) return;
    set({ selection });
    itemSelection.getState().setTimeRange(selection ? { start: selection.start, end: selection.end } : null);
  },
  setClipboard: (clipboard) => set({ clipboard }),
  notify: (message) => set({ notice: message === null ? null : { id: nextNotice++, message } }),
}));

export function clearTimeSelection(): void {
  useTimeSelection.getState().setSelection(null);
}

/** Reset (tests). */
export function resetTimeSelection(): void {
  useTimeSelection.setState({ selection: null, clipboard: null, notice: null });
}
