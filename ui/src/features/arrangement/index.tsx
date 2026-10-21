// The app shell (ui/src/app/App.tsx) mounts `ArrangementView`: keep it prop-less (read state via hooks).

/** Arrangement view: tracks, clips (move/resize/split/loop), canvas waveforms. */
export { ArrangementView } from "./ArrangementView";
export { useKeepTrackHeights } from "./keptHeights";
