// OWNERSHIP: the `plugins` node owns `ui/src/features/plugins/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) already mounts
// `PluginBrowser`: keep this export name and keep it prop-less (read state via hooks).
// `PluginDeviceControls` is mounted in every device header by `features/devices/DeviceView`.
import "./plugins.css";
import { PluginBrowserView } from "./PluginBrowser";

/** Plugin browser: scanned CLAP/VST3/VST2/AU plugins, format filter, search, rescan, click to insert (desktop only). */
export function PluginBrowser() {
  return <PluginBrowserView />;
}

export { PluginDeviceControls, type PluginDeviceControlsProps } from "./PluginDeviceControls";
/** Settings > Plugins: plugin folders (system + user, format filters), Rescan / Full rescan. */
export { PluginFoldersPanel } from "./PluginFolders";
