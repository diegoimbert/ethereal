// Project versions (CONTRACTS.md §13.11): the versions dialog (save, compare, restore,
// rename, delete) and the crash-recovery dialog shown at startup. The app shell mounts
// `VersionsRoot`; anything can open the versions dialog with `openVersions()`, and the
// "save snapshot…" name prompt (a named version) with `openSnapshotPrompt()`.
import "./versions.css";

export { openSnapshotPrompt, openVersions, useSnapshotPrompt, useVersionsDialog } from "./store";
export { SnapshotPrompt } from "./SnapshotPrompt";
export { RecoveryDialog } from "./RecoveryDialog";
export { VersionsDialog } from "./VersionsDialog";
export { VersionsRoot } from "./VersionsRoot";
