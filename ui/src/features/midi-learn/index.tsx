// The left rail (`app/shell/LeftRail.tsx`) mounts `MidiLearnPanel` (tab "midi"): keep it
// prop-less (read state via hooks). The shell mounts `MidiLearnRoot` once (app-wide learn
// events and MIDI mode, whether or not the tab is open).
import "./midi-learn.css";

/** MIDI learn: map controllers to params, mixer and transport. */
export { MidiLearnPanel } from "./MidiLearnPanel";
export { MidiLearnRoot } from "./MidiLearnRoot";
