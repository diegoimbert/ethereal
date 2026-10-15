import { DrumRackTab } from "./DrumRackView";

/** Drum rack pads, pad chains, choke groups of the selected track (the tests' harness). */
export function DrumRackView() {
  return <DrumRackTab />;
}

/** A drum rack device's own editor (pads, pad chains), for its device window. */
export { RackEditor as DrumRackEditor } from "./DrumRackView";
export { isDrumRack } from "./padUtils";
export { useDrumFocus } from "./focusStore";
export { SamplerModeSwitch, SliceEditor } from "./SliceEditor";
export { slicePayloadToNewTrack, slicePayloadToPads, sliceMediaToNewTrack } from "./sliceActions";
export { sliceSampleEntries, withSliceClipEntries } from "./menus";
