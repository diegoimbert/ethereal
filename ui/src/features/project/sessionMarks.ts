// Recents "Local copy" badge (base-114): the backup the engine keeps when a collaboration
// session replaced a differing stored copy (`collab_backup`: the name gets a " (local copy)"
// suffix). Sharing marks ("Shared", "From …") come from the engine (`ProjectSummary.share`,
// read from `share.json`; recents-shared), not from this device.

/** Suffix the engine gives the backup it keeps of a local version (collab/mod.rs). */
export const LOCAL_COPY_SUFFIX = " (local copy)";

/** `"Song (local copy)"` → `{ base: "Song", localCopy: true }`. */
export function splitLocalCopy(name: string): {
  base: string;
  localCopy: boolean;
} {
  return name.endsWith(LOCAL_COPY_SUFFIX) && name.length > LOCAL_COPY_SUFFIX.length
    ? { base: name.slice(0, -LOCAL_COPY_SUFFIX.length), localCopy: true }
    : { base: name, localCopy: false };
}
