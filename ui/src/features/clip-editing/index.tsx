// OWNERSHIP: the `clip-editing` node owns `ui/src/features/clip-editing/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) already mounts
// `MarkerLane` (main panel, `data-slot="markers"` above the arrangement): keep the export names and keep them prop-less (read state via hooks).

/**
 * Arrangement markers; clip fades, crossfades and reverse live in the arrangement.
 *
 * Inline slot: renders nothing until implemented, so the shell layout is unchanged.
 */
export function MarkerLane() {
  return null;
}

// The fade law mirror (`fadeGain`) lives in `./fades` (import it from there; react-refresh
// wants component-only index files).
