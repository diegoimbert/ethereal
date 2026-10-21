// The app shell (ui/src/app/App.tsx) mounts `RecordingControls`: keep it prop-less (read state via hooks).
import "./recording.css";

/** Recording: arm, input monitoring, audio + MIDI record. */
export { RecordingControls, unsupportedTooltip } from "./RecordingControls";
export { TrackRecordSetup } from "./TrackInputs";
export { bindRecordingSetup, loadRecordingSetup, recordingSetupKey, useKeepRecordingSetup } from "./keep";
