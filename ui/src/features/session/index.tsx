// OWNERSHIP: the `ui-session` node owns `ui/src/features/session/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) already mounts
// `SessionView`: keep this export name and keep it prop-less (read state via hooks).

/** Session view: clip grid, launch/stop, scenes. */
export function SessionView() {
  return (
    <div className="eth-feature-placeholder" data-feature="session">
      <strong>SessionView</strong>
      <span>Session view: clip grid, launch/stop, scenes.</span>
      <span className="eth-feature-placeholder__owner">owner: ui-session</span>
    </div>
  );
}
