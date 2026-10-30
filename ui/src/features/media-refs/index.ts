// Samples referenced in place: missing media, Relink, Collect All (v0.2;
// CONTRACTS.md §12.9), and the media manager (every sample, what uses it, Remove).
import "./mediaRefs.css";

export { MediaRefsRoot, MissingMediaStatus } from "./MediaRefsRoot";
export { MediaManagerDialog } from "./MediaManagerDialog";
export { RelinkDialog } from "./RelinkDialog";
export { isUnused, mediaUsage, type MediaUsage } from "./usage";
export { mediaRefClipEntries, mediaRefCommands, withMediaRefClipEntries, type MediaRefPaletteCommand } from "./menus";
export { isSampleDrag, readSampleDrop, relinkDrop, sampleDropSource, useSampleDrop, type SampleDrop } from "./drop";
export { hasFolderPicker, locationLabel, pickRelinkSource, sourceLabel, stageUpload, type FolderPickerHost } from "./sources";
export {
  applyMediaRefEvent,
  closeMediaManager,
  closeRelink,
  collectAll,
  openMediaManager,
  openRelink,
  refreshMissing,
  relink,
  removeMedia,
  resetMediaRefs,
  search,
  useMediaMissing,
  useMediaRefs,
} from "./store";
