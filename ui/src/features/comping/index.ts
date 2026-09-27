/**
 * Takes and comping (v0.2, `comping`): expandable take lanes under audio/MIDI tracks, swipe
 * comping, audition, flatten, next/previous take shortcuts. Mounted by the arrangement
 * (`TrackRow.tsx`, `ArrangementView.tsx`); the model helpers are shared with the mock.
 */

export { handleTakeKey, trackTakeEntries } from "./actions";
export { CompLayer } from "./CompLayer";
export { TAKE_LANE_HEIGHT, useTakesHeight } from "./height";
export { TakeLanes } from "./TakeLanes";
export { TakesToggle } from "./TakesToggle";
export { useCompingUi } from "./store";
