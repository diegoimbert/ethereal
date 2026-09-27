// OWNERSHIP: the `drum-rack` node owns `ui/src/features/drum-rack/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) already mounts
// `DrumRackView` (detail tab "drum-rack"): keep the export names and keep them prop-less (read state via hooks).

/** Drum rack pads, pad chains, choke groups, sampler slicing. */
export function DrumRackView() {
  return (
    <div className="eth-feature-placeholder" data-feature="drum-rack">
      <strong>DrumRackView</strong>
      <span>Drum rack pads, pad chains, choke groups, sampler slicing.</span>
      <span className="eth-feature-placeholder__owner">owner: drum-rack</span>
    </div>
  );
}
