import { RecoveryDialog } from "./RecoveryDialog";
import { SnapshotPrompt } from "./SnapshotPrompt";
import { VersionsDialog } from "./VersionsDialog";

/** The versions dialogs (mounted once by the app shell). */
export function VersionsRoot() {
  return (
    <>
      <VersionsDialog />
      <RecoveryDialog />
      <SnapshotPrompt />
    </>
  );
}
