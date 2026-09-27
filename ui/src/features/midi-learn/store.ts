// MIDI learn UI state: the MIDI-mode switch (local) and the engine's learn state, mirrored
// from `Event::MidiMap` (learn target, last learned mapping, last input activity).
import { create } from "zustand";
import type { Event, MidiMapEvent, MidiMappingId, MidiMapTarget, MidiSource } from "@/generated";

export interface MidiLearnState {
  /** MIDI mode: mappable controls are highlighted and clicking one learns it. */
  enabled: boolean;
  /** Target the engine is learning (`LearnChanged`). */
  learning: MidiMapTarget | null;
  /** Last mapping created by learning (`Learned`), highlighted in the list. */
  learned: MidiMappingId | null;
  /** Last incoming source (`Activity`, ~10 Hz) and a counter bumped on each report. */
  activity: MidiSource | null;
  activitySeq: number;
  setEnabled(enabled: boolean): void;
  /** Apply one engine event (ignores non-MIDI-map events). */
  onEvent(event: Event): void;
  reset(): void;
}

const INITIAL = { enabled: false, learning: null, learned: null, activity: null, activitySeq: 0 } as const;

export function reduceMidiEvent(
  s: Pick<MidiLearnState, "learning" | "learned" | "activity" | "activitySeq">,
  e: MidiMapEvent,
): Partial<MidiLearnState> {
  switch (e.type) {
    case "LearnChanged":
      return { learning: e.target };
    case "Learned":
      return { learned: e.mapping };
    case "Activity":
      return { activity: e.source, activitySeq: s.activitySeq + 1 };
  }
}

export const useMidiLearnStore = create<MidiLearnState>()((set, get) => ({
  ...INITIAL,
  setEnabled: (enabled) => set({ enabled }),
  onEvent: (event) => {
    if (event.type === "MidiMap") set(reduceMidiEvent(get(), event.event));
    // A new project starts without a learn in progress.
    if (event.type === "ProjectLoaded") set({ learning: null, learned: null });
  },
  reset: () => set({ ...INITIAL }),
}));
