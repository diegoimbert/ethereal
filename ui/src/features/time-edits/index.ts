/**
 * Time-selection edits in the arrangement (v0.2, `time-edits`; CONTRACTS.md §12.3): the
 * time selection (a marquee drag over the lanes), its overlay and context menu, and the
 * time-edit shortcuts (see `actions.ts`).
 */
export {
  bindTimeSelection,
  canRun,
  clipboardAction,
  inTimeSelection,
  runTimeAction,
  timeActionForKey,
  timeSelectionMenu,
  type TimeAction,
  type TimeActionContext,
} from "./actions";
export { coversWholeSong, expandTracks, isTimeTrack, pasteTracks, selectionFromRect, timeSelection } from "./commands";
export {
  advanceCursor,
  bindPlayFrom,
  insertMarker,
  insertPoint,
  pasteTarget,
  placeInsertMarker,
  selectionCursor,
  setPlayStart,
  stopButton,
  playOrStopHere,
  playOrStopToStart,
} from "./marker";
export { clearTimeSelection, isMarker, rangeOf, trackRangeOf, resetTimeSelection, useTimeSelection, type TimeRangeSelection } from "./store";
export { TimeSelectionLayer } from "./TimeSelectionLayer";
export { useArrangementTimeEdits, type ArrangementTimeEdits } from "./useArrangementTimeEdits";
