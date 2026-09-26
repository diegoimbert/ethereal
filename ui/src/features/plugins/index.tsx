// OWNERSHIP: the `plugins` node owns `ui/src/features/plugins/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) already mounts
// `PluginBrowser`: keep this export name and keep it prop-less (read state via hooks).

/** Plugin browser: CLAP plugins, sandbox toggle. */
export function PluginBrowser() {
  return (
    <div className="eth-feature-placeholder" data-feature="plugins">
      <strong>PluginBrowser</strong>
      <span>Plugin browser: CLAP plugins, sandbox toggle.</span>
      <span className="eth-feature-placeholder__owner">owner: plugins</span>
    </div>
  );
}
