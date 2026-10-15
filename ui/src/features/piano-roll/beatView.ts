import { create } from "zustand";
import type { TrackId } from "@/generated";

/**
 * Which editor a Drum Rack track's clips open in: the step sequencer (`BeatGrid`, default) or
 * the piano roll (per track, this session).
 */
interface BeatViewState {
  pianoRoll: ReadonlySet<TrackId>;
  setPianoRoll(track: TrackId, on: boolean): void;
}

export const useBeatView = create<BeatViewState>()((set) => ({
  pianoRoll: new Set(),
  setPianoRoll: (track, on) =>
    set((s) => {
      const next = new Set(s.pianoRoll);
      if (on) next.add(track);
      else next.delete(track);
      return { pianoRoll: next };
    }),
}));
