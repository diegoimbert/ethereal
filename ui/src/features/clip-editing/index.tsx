// OWNERSHIP: the `clip-editing` node owns `ui/src/features/clip-editing/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) already mounts
// `MarkerLane` (main panel, `data-slot="markers"` above the arrangement): keep the export names and keep them prop-less (read state via hooks).

/** Arrangement markers (`MarkerLane`); clip fades, crossfades and reverse live in the arrangement clips (`ClipFades`). */
export { MarkerLane } from "./MarkerLane";
export { ClipFades, ReversedBadge } from "./ClipFades";

// The fade law mirror (`fadeGain`) lives in `./fades` and the command/menu helpers in
// `./clipEditing` (import them from there; react-refresh wants component-only index files).
