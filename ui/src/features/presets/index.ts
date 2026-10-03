// OWNERSHIP: the `presets` node owns `ui/src/features/presets/**`. `PresetMenu` is mounted
// in every device header by `features/devices/DeviceView` (keep its name and props).
import "./presets.css";

export { PresetMenu, type PresetMenuProps } from "./PresetMenu";
export { loadPresetCommand, presetDeviceOf, useCurrentPresets } from "./model";
