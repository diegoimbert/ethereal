// OWNERSHIP: the `ui-shell` node owns `ui/src/features/browser/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) already mounts
// `Browser`: keep this export name and keep it prop-less (read state via hooks).

/** Browser: files, samples, devices. */
export function Browser() {
  return (
    <div className="eth-feature-placeholder" data-feature="browser">
      <strong>Browser</strong>
      <span>Browser: files, samples, devices.</span>
      <span className="eth-feature-placeholder__owner">owner: ui-shell</span>
    </div>
  );
}
