// OWNERSHIP: the `ui-automation` node owns `ui/src/features/automation/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) already mounts
// `AutomationLanes`: keep this export name and keep it prop-less (read state via hooks).

/** Automation: SVG breakpoint lanes, curve types. */
export function AutomationLanes() {
  return (
    <div className="eth-feature-placeholder" data-feature="automation">
      <strong>AutomationLanes</strong>
      <span>Automation: SVG breakpoint lanes, curve types.</span>
      <span className="eth-feature-placeholder__owner">owner: ui-automation</span>
    </div>
  );
}
