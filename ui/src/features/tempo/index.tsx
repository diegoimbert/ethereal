// OWNERSHIP: the `tempo-metronome` node owns `ui/src/features/tempo/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) already mounts
// `TempoEditor` (detail tab "tempo") and `MetronomeSettings` (top bar,
// `data-slot="metronome"`): keep the export names and keep them prop-less (read state via hooks).

/** Tempo map and time-signature changes (add, move, remove, ramps). */
export { TempoEditor } from "./TempoEditor";
/** Metronome on/off, volume, accent and sound (top bar popover). */
export { MetronomeSettings } from "./MetronomeSettings";
