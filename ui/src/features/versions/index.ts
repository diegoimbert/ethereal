// OWNERSHIP: the `project-versions` node owns `ui/src/features/versions/**`.
// Project versions (CONTRACTS.md §13.11): the versions dialog (save, compare, restore,
// rename, delete) and the crash-recovery dialog shown at startup. The app shell mounts
// `VersionsRoot`; anything can open the versions dialog with `openVersions()`.
import "./versions.css";

export { openVersions, useVersionsDialog } from "./store";
export { RecoveryDialog } from "./RecoveryDialog";
export { VersionsDialog } from "./VersionsDialog";
export { VersionsRoot } from "./VersionsRoot";
