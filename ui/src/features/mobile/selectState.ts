/**
 * Multi-select on touch (#522, the design's §5): no Shift or Ctrl on a phone, so "select more"
 * (the clip bar) turns taps into toggles and a drag on empty lane space into a selection box,
 * until "done" (or nothing is selected any more).
 */
import { create } from "zustand";

export const useTouchSelect = create<{ multi: boolean }>()(() => ({ multi: false }));

export const setMultiSelect = (multi: boolean): void => useTouchSelect.setState({ multi });
