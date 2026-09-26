// OWNERSHIP: the `ui-mixer` node owns `ui/src/features/devices/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) already mounts
// `DeviceChain`: keep this export name and keep it prop-less (read state via hooks).
import { DeviceChainView } from "./DeviceChain";

/** Device chain of the selected track with generic parameter UI. */
export function DeviceChain() {
  return <DeviceChainView />;
}
