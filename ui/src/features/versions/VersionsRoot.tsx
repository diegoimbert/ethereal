import { RecoveryDialog } from "./RecoveryDialog";
import { VersionsDialog } from "./VersionsDialog";

/** Both dialogs (mounted once by the app shell). */
export function VersionsRoot() {
  return (
    <>
      <VersionsDialog />
      <RecoveryDialog />
    </>
  );
}
