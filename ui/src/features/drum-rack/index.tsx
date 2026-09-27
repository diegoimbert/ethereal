// OWNERSHIP: the `drum-rack` node owns `ui/src/features/drum-rack/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) already mounts
// `DrumRackView` (detail tab "drum-rack"): keep the export names and keep them prop-less (read state via hooks).
import { DrumRackTab } from "./DrumRackView";

/** Drum rack pads, pad chains, choke groups, sampler slicing (selected track). */
export function DrumRackView() {
  return <DrumRackTab />;
}
