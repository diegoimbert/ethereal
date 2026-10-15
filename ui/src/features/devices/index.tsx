// The inspector mounts `DeviceChain` (`track`, `layout="stack"`); prop-less, it shows the
// selected track's chain.
import { DeviceChainView, type DeviceChainProps } from "./DeviceChain";

/**
 * Device chain with generic parameter UI: the selected track's by default (prop-less), or
 * a given track's, in a row or stacked (the inspector) layout.
 */
export function DeviceChain(props: DeviceChainProps = {}) {
  return <DeviceChainView {...props} />;
}
