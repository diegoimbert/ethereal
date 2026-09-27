// OWNERSHIP: the `groove` node owns `ui/src/features/groove/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) already mounts
// `GroovePanel` (detail tab "groove"): keep the export names and keep them prop-less (read state via hooks).

/** Groove: quantize swing, humanize, project swing. */
export function GroovePanel() {
  return (
    <div className="eth-feature-placeholder" data-feature="groove">
      <strong>GroovePanel</strong>
      <span>Groove: quantize swing, humanize, project swing.</span>
      <span className="eth-feature-placeholder__owner">owner: groove</span>
    </div>
  );
}
