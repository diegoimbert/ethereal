// OWNERSHIP: the `ui-piano-roll` node owns `ui/src/features/piano-roll/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) already mounts
// `PianoRoll`: keep this export name and keep it prop-less (read state via hooks).

/** Piano roll: note editing, velocity lane, quantize. */
export function PianoRoll() {
  return (
    <div className="eth-feature-placeholder" data-feature="piano-roll">
      <strong>PianoRoll</strong>
      <span>Piano roll: note editing, velocity lane, quantize.</span>
      <span className="eth-feature-placeholder__owner">owner: ui-piano-roll</span>
    </div>
  );
}
