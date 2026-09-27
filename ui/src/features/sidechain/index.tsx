// OWNERSHIP: the `sidechain` node owns `ui/src/features/sidechain/**`.
// Only edit files inside this folder. `SidechainSelector` is mounted in every device header
// by `features/devices/DeviceView` (next to `PluginDeviceControls`): keep its name and props.
import type { Device } from "@/generated";
import { useDescriptor } from "@/features/devices/descriptors";
import { useSend } from "@/features/devices/gesture";
import { Select, type SelectOption } from "@/kit";
import { useProject } from "@/state";
import { cmd } from "@/transport";
import { sidechainSources } from "./routing";

export interface SidechainSelectorProps {
  device: Device;
}

const NONE = "";

/**
 * Sidechain source selector (`Device::SetSidechain`, one undo step per change). Renders
 * nothing for devices without a sidechain input (`DeviceDescriptor.sidechain_inputs == 0`)
 * and for drum-rack pad devices. Lists every track and return except master and the
 * device's own track; sources that would close a routing cycle are disabled.
 */
export function SidechainSelector({ device }: SidechainSelectorProps) {
  const { descriptor } = useDescriptor(device);
  const project = useProject();
  const send = useSend();
  if (!descriptor || descriptor.sidechain_inputs === 0 || device.pad !== null || !project) return null;

  const options: SelectOption<string>[] = [
    { value: NONE, label: "No sidechain" },
    ...sidechainSources(project, device).map(({ track, cycle }) => ({
      value: track.id,
      label: cycle ? `${track.name} (would loop)` : track.name,
      disabled: cycle,
    })),
  ];
  return (
    <Select
      size="sm"
      className="eth-sidechain"
      aria-label={`Sidechain source for ${device.name}`}
      title="Sidechain source: the track whose signal drives this device's detector"
      options={options}
      value={device.sidechain ?? NONE}
      onChange={(v) => void send(cmd("Device", { type: "SetSidechain", device: device.id, source: v === NONE ? null : v }))}
    />
  );
}
