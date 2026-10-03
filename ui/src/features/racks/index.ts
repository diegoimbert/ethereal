// OWNERSHIP: the `racks-modulation` node owns `ui/src/features/racks/**`.
// `RackPanel` is mounted by `features/devices/DeviceView` for instrument / audio effect /
// MIDI effect racks (below the rack's macros); `groupEntries` feeds its header menu.
export { RackPanel, type RackPanelProps, type RenderDeviceProps } from "./RackPanel";
export { fitsChain, isChainRack } from "./model";
export { groupEntries } from "./group";
