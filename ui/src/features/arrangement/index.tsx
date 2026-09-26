// OWNERSHIP: the `ui-arrangement` node owns `ui/src/features/arrangement/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) already mounts
// `ArrangementView`: keep this export name and keep it prop-less (read state via hooks).

/** Arrangement view: tracks, clips (move/resize/split/loop), canvas waveforms. */
export function ArrangementView() {
  return (
    <div className="eth-feature-placeholder" data-feature="arrangement">
      <strong>ArrangementView</strong>
      <span>Arrangement view: tracks, clips (move/resize/split/loop), canvas waveforms.</span>
      <span className="eth-feature-placeholder__owner">owner: ui-arrangement</span>
    </div>
  );
}
