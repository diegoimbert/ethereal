/**
 * Soloed drum pads. Pad solo is runtime engine state (`DrumRack::SetPadSolo`: not saved,
 * not undoable, no patch), so the view remembers what it asked for. Session-local.
 */

import { create } from "zustand";
import type { DrumPadId } from "@/generated";

interface SoloState {
  soloed: ReadonlySet<DrumPadId>;
  set(pad: DrumPadId, solo: boolean): void;
  reset(): void;
}

export const useDrumSolo = create<SoloState>()((set) => ({
  soloed: new Set(),
  set: (pad, solo) =>
    set((s) => {
      const next = new Set(s.soloed);
      if (solo) next.add(pad);
      else next.delete(pad);
      return { soloed: next };
    }),
  reset: () => set({ soloed: new Set() }),
}));
