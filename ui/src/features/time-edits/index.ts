/**
 * Time-selection edits in the arrangement (v0.2, `time-edits`; CONTRACTS.md §12.3): the
 * time selection (a marquee drag over the lanes), its overlay and context menu, and the
 * time-edit shortcuts (see `actions.ts`).
 */
export {
  bindTimeSelection,
  canRun,
  inTimeSelection,
  runTimeAction,
  timeActionForKey,
  timeSelectionMenu,
  type TimeAction,
  type TimeActionContext,
} from "./actions";
export { coversWholeSong, expandTracks, pasteTracks, selectionFromRect, timeSelection } from "./commands";
export { clearTimeSelection, resetTimeSelection, useTimeSelection, type TimeRangeSelection } from "./store";
export { TimeEditNotice, TimeSelectionLayer } from "./TimeSelectionLayer";
export { useArrangementTimeEdits, type ArrangementTimeEdits } from "./useArrangementTimeEdits";
