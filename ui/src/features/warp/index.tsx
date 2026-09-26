// OWNERSHIP: the `warp` node owns `ui/src/features/warp/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) already mounts
// `WarpEditor`: keep this export name and keep it prop-less (read state via hooks).

/** Warp editor: warp markers, BPM detection. */
export function WarpEditor() {
  return (
    <div className="eth-feature-placeholder" data-feature="warp">
      <strong>WarpEditor</strong>
      <span>Warp editor: warp markers, BPM detection.</span>
      <span className="eth-feature-placeholder__owner">owner: warp</span>
    </div>
  );
}
