// OWNERSHIP: the `midi-learn` node owns `ui/src/features/midi-learn/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) already mounts
// `MidiLearnPanel` (sidebar tab "midi"): keep the export names and keep them prop-less (read state via hooks).
import "./midi-learn.css";

/** MIDI learn: map controllers to params, mixer and transport. */
export { MidiLearnPanel } from "./MidiLearnPanel";
