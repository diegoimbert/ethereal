/**
 * Groups, buses, track input taps and VCAs (v0.2, CONTRACTS.md §12.10): Cmd+G / Ungroup,
 * VCA assignment, track-to-track input. Mounted by the arrangement and the inspector.
 */
import "./groups.css";

export { assignVca, groupsTrackMenu, groupTracks, newVcaFor, ungroupSelected, ungroupTrack, useUngroupConfirm } from "./actions";
export { GroupsRoutingRows, TrackInputSelect, VcaSelect, VcaSummary } from "./controls";
export {
  assignedTo,
  childrenOf,
  groupableSelection,
  groupCommand,
  inputSources,
  isWithin,
  takesTrackInput,
  ungroupLosses,
  vcaTargets,
  vcaTracks,
  type Tracks,
} from "./model";
export { UngroupConfirmDialog } from "./UngroupConfirmDialog";
export { VcaLane } from "./VcaLane";
