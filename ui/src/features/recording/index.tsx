// OWNERSHIP: the `recording` node owns `ui/src/features/recording/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) already mounts
// `RecordingControls`: keep this export name and keep it prop-less (read state via hooks).
import "./recording.css";

/** Recording: arm, input monitoring, audio + MIDI record. */
export { RecordingControls, UNSUPPORTED_TOOLTIP } from "./RecordingControls";
