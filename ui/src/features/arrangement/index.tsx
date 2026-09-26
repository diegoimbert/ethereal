// OWNERSHIP: the `ui-arrangement` node owns `ui/src/features/arrangement/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) already mounts
// `ArrangementView`: keep this export name and keep it prop-less (read state via hooks).

/** Arrangement view: tracks, clips (move/resize/split/loop), canvas waveforms. */
export { ArrangementView } from "./ArrangementView";
