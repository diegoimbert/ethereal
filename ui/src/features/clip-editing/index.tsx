// The app shell (ui/src/app/App.tsx) mounts `MarkerLane` (main panel, `data-slot="markers"` above
// the arrangement): keep it prop-less (read state via hooks).

/** Arrangement markers (`MarkerLane`); clip fades, crossfades and reverse are drawn by the arranger canvas (arrangement/gl/layers). */
export { MarkerLane } from "./MarkerLane";

// The fade law mirror (`fadeGain`) lives in `./fades` and the command/menu helpers in
// `./clipEditing` (import them from there; react-refresh wants component-only index files).
