// `PluginDeviceControls` is mounted in every device header by `features/devices/DeviceView`;
// scanned plugins are listed by the device picker (`@/features/devices/DeviceLibrary`).
import "./plugins.css";

export { PluginDeviceControls, pluginMenuEntries, setPluginWindow, type PluginDeviceControlsProps } from "./PluginDeviceControls";
/** Settings > Plugins: plugin folders (system + user, format filters), Rescan / Full rescan, failed plugins (retry). */
export { PluginFoldersPanel } from "./PluginFolders";
/** Plugins loading (a big project opening): a bar per track header, a status bar item. */
export { PluginLoadingStatus, TrackPluginLoading } from "./PluginLoading";
