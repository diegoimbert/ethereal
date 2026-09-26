// OWNERSHIP: the `ui-shell` node owns `ui/src/features/project/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) already mounts
// `ProjectMenu`: keep this export name and keep it prop-less (read state via hooks).

/** Project menu: new/open/save .ether. */
export function ProjectMenu() {
  return (
    <div className="eth-feature-placeholder" data-feature="project">
      <strong>ProjectMenu</strong>
      <span>Project menu: new/open/save .ether.</span>
      <span className="eth-feature-placeholder__owner">owner: ui-shell</span>
    </div>
  );
}
