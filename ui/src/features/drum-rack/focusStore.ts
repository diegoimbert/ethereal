import { create } from "zustand";
import type { DeviceId } from "@/generated";

/**
 * A request to show one pad in a Drum Rack's editor (e.g. a click on a pad's row in the beat
 * editor): the rack's editor selects that note and switches to its bank. `seq` makes a repeat
 * of the same request apply again.
 */
interface DrumFocusState {
  focus: { rack: DeviceId; note: number; seq: number } | null;
  focusPad(rack: DeviceId, note: number): void;
}

export const useDrumFocus = create<DrumFocusState>()((set, get) => ({
  focus: null,
  focusPad: (rack, note) => set({ focus: { rack, note, seq: (get().focus?.seq ?? 0) + 1 } }),
}));
