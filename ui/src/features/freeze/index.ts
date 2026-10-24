// Freeze, flatten, bounce and consolidate (v0.2; CONTRACTS.md §12.3).
import "./freeze.css";

export { FreezeHeaderStatus } from "./FreezeHeaderStatus";
export {
  chainRoot,
  descendantsOf,
  frozenAncestor,
  frozenChainLength,
  inertTitle,
  isDeviceFrozen,
  isFrozenTrack,
  useDeviceFrozen,
  useFrozenAncestor,
} from "./inert";
export {
  bounceIfAllowed,
  bounceRangeIfAllowed,
  consolidateIfAllowed,
  consolidateRangeIfAllowed,
  freezeClipEntries,
  freezeDeviceEntries,
  freezeRangeEntries,
  freezeTrackEntries,
  withFreezeClipEntries,
  withFreezeTrackEntries,
} from "./menus";
export {
  bounceClips,
  bounceRange,
  cancelRenderJob,
  consolidateClips,
  consolidateRange,
  flattenTracks,
  freezeTracks,
  freezeUpTo,
  jobOfTrack,
  runRenderJob,
  unfreezeTracks,
  useFreezeUi,
  type RenderJob,
  type RenderKind,
} from "./store";
