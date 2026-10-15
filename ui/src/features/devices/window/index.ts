// base-153: factory device windows (a built-in device's own UI opens in a floating window,
// like a plugin's editor window) and the header's window toggle shared by every device.
export { DeviceEditor, DeviceWindowsRoot } from "./DeviceWindow";
export { DeviceWindowButton, DeviceWindowToggle, type DeviceWindowButtonProps } from "./DeviceWindowButton";
export { deviceWindowId, resetDeviceWindows, useDeviceWindows } from "./store";
export { setDeviceWindow, showDeviceWindow, useDeviceWindow, type DeviceWindowControl } from "./useDeviceWindow";
