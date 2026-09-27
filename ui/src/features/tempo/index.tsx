// OWNERSHIP: the `tempo-metronome` node owns `ui/src/features/tempo/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) already mounts
// `TempoEditor` (detail tab "tempo") and `MetronomeSettings` (top bar,
// `data-slot="metronome"`): keep the export names and keep them prop-less (read state via hooks).

/** Tempo map and time-signature changes (add, move, remove, ramps). */
export function TempoEditor() {
  return (
    <div className="eth-feature-placeholder" data-feature="tempo">
      <strong>TempoEditor</strong>
      <span>Tempo map and time-signature changes (add, move, remove, ramps).</span>
      <span className="eth-feature-placeholder__owner">owner: tempo-metronome</span>
    </div>
  );
}

/**
 * Metronome volume, accent and sound.
 *
 * Inline slot: renders nothing until implemented, so the shell layout is unchanged.
 */
export function MetronomeSettings() {
  return null;
}
