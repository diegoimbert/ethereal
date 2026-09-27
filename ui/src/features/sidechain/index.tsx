// OWNERSHIP: the `sidechain` node owns `ui/src/features/sidechain/**`.
// Only edit files inside this folder. `SidechainSelector` is mounted in every device header
// by `features/devices/DeviceView` (next to `PluginDeviceControls`): keep its name and props.
import type { Device } from "@/generated";

export interface SidechainSelectorProps {
  device: Device;
}

/**
 * Sidechain source selector (`Device::SetSidechain`). Renders nothing for devices without a
 * sidechain input (`DeviceDescriptor.sidechain_inputs == 0`). Placeholder: renders nothing.
 */
export function SidechainSelector({ device }: SidechainSelectorProps) {
  void device;
  return null;
}
