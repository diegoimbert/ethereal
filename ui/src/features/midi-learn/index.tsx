// OWNERSHIP: the `midi-learn` node owns `ui/src/features/midi-learn/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) already mounts
// `MidiLearnPanel` (sidebar tab "midi"): keep the export names and keep them prop-less (read state via hooks).

/** MIDI learn: map controllers to params, mixer and transport. */
export function MidiLearnPanel() {
  return (
    <div className="eth-feature-placeholder" data-feature="midi-learn">
      <strong>MidiLearnPanel</strong>
      <span>MIDI learn: map controllers to params, mixer and transport.</span>
      <span className="eth-feature-placeholder__owner">owner: midi-learn</span>
    </div>
  );
}
