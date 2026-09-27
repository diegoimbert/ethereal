// OWNERSHIP: the `groups-buses` node owns `ui/src/features/groups/**`.
/**
 * Groups, buses, track input taps and VCAs (v0.2, CONTRACTS.md §12.10): Cmd+G / Ungroup,
 * VCA assignment, track-to-track input. Mounted by the arrangement and the mixer.
 */
import "./groups.css";

export {
  assignVca,
  groupShortcut,
  groupsTrackMenu,
  groupTracks,
  newVcaFor,
  ungroupSelected,
  ungroupTrack,
  useUngroupConfirm,
} from "./actions";
export { TrackInputSelect, VcaSelect, VcaSummary } from "./controls";
export {
  assignedTo,
  groupableSelection,
  groupCommand,
  inputSources,
  tapLabel,
  takesTrackInput,
  ungroupLosses,
  vcaTargets,
  vcaTracks,
} from "./model";
export { UngroupConfirmDialog } from "./UngroupConfirmDialog";
export { VcaLane } from "./VcaLane";
