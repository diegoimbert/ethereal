// The app shell (ui/src/app/App.tsx) mounts `ExportDialog` (top bar, `data-slot="export"`): keep it
// prop-less (read state via hooks).
import "./export.css";

/** Export audio: mix or stems, WAV/FLAC, range, normalize, tail; downloads on the web. */
export { ExportDialog } from "./ExportDialog";
export { useExportAudio } from "./store";
