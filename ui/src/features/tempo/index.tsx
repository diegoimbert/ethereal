// The top bar's metronome button opens `MetronomePanel` (a right-click): keep it prop-less. The tempo map is edited on the master track's "Tempo" automation lane
// (`TempoAutomationLane`, mounted by `features/automation`) and on the ruler.

/** The tempo map as a lane (add, move, remove points, ramps) and the BPM at the playhead. */
export { PlayheadBpm, TempoAutomationLane, type TempoAutomationLaneProps } from "./TempoAutomationLane";
/** Metronome on/off, volume, accent and sound (top bar popover). */
export { MetronomePanel } from "./MetronomeSettings";
/** The timeline ruler with tempo-map editing. */
export { TempoRuler } from "./TempoRuler";
/** Tempo and time-signature changes for a lane outside the ruler (the arrangement's marker lane). */
export { TempoMapMarkers } from "./RulerTempoMarkers";
