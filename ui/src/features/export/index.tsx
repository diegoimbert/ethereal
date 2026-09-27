// OWNERSHIP: the `export` node owns `ui/src/features/export/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) already mounts
// `ExportDialog` (top bar, `data-slot="export"`): keep the export names and keep them prop-less (read state via hooks).

/**
 * Export audio: mix or stems, WAV/FLAC, range, normalize, tail; downloads on the web.
 *
 * Inline slot: renders nothing until implemented, so the shell layout is unchanged.
 */
export function ExportDialog() {
  return null;
}
