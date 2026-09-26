// OWNERSHIP: the `ui-shell` node owns `ui/src/features/transport-bar/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) already mounts
// `TransportBar`: keep this export name and keep it prop-less (read state via hooks).

/** Transport bar: play/stop/record, tempo, time signature, position. */
export function TransportBar() {
  return (
    <div className="eth-feature-placeholder" data-feature="transport-bar">
      <strong>TransportBar</strong>
      <span>Transport bar: play/stop/record, tempo, time signature, position.</span>
      <span className="eth-feature-placeholder__owner">owner: ui-shell</span>
    </div>
  );
}
