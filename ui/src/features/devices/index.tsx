// OWNERSHIP: the `ui-mixer` node owns `ui/src/features/devices/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) already mounts
// `DeviceChain`: keep this export name and keep it prop-less (read state via hooks).
import { DeviceChainView, type DeviceChainProps } from "./DeviceChain";

/**
 * Device chain with generic parameter UI: the selected track's by default (prop-less), or
 * a given track's, in a row or stacked (the inspector) layout.
 */
export function DeviceChain(props: DeviceChainProps = {}) {
  return <DeviceChainView {...props} />;
}
