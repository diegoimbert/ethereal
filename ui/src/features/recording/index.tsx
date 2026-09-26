// OWNERSHIP: the `recording` node owns `ui/src/features/recording/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) already mounts
// `RecordingControls`: keep this export name and keep it prop-less (read state via hooks).

/** Recording: arm, input monitoring, audio + MIDI record. */
export function RecordingControls() {
  return (
    <div className="eth-feature-placeholder" data-feature="recording">
      <strong>RecordingControls</strong>
      <span>Recording: arm, input monitoring, audio + MIDI record.</span>
      <span className="eth-feature-placeholder__owner">owner: recording</span>
    </div>
  );
}
