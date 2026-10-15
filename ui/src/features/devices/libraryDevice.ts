// A device type picked in the device library (`DeviceLibrary`), to add to a track's chain.
import type { BuiltinDeviceType, DeviceCategory, PluginFormat } from "@/generated";

export type LibraryDevice =
  | { type: "Builtin"; builtin: BuiltinDeviceType }
  | {
      type: "Plugin";
      plugin_id: string;
      format: PluginFormat;
      /** The listed name: picks one of several plugins sharing `(format, plugin_id)` (VST2 uniqueID collisions). */
      name?: string;
      /** Another listed plugin has the same `(format, plugin_id)`: its library key adds the name. */
      shared?: boolean;
    };

export interface LibraryDevicePayload {
  name: string;
  category: DeviceCategory;
  device: LibraryDevice;
}
