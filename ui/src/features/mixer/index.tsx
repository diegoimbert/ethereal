// OWNERSHIP: the `ui-mixer` node owns `ui/src/features/mixer/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) already mounts
// `Mixer`: keep this export name and keep it prop-less (read state via hooks).

/** Mixer: strips, meters, sends. */
export function Mixer() {
  return (
    <div className="eth-feature-placeholder" data-feature="mixer">
      <strong>Mixer</strong>
      <span>Mixer: strips, meters, sends.</span>
      <span className="eth-feature-placeholder__owner">owner: ui-mixer</span>
    </div>
  );
}
