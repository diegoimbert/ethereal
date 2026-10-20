import { ExtensionRoots } from "@/app/ExtensionRoots";
import { ImportStatus } from "@/features/import/ImportStatus";
import { MissingPluginsStatus } from "@/features/import-resolver/MissingItems";
import { MissingMediaStatus } from "@/features/media-refs/MediaRefsRoot";
import "./statusBar.css";

/**
 * The thin bar along the bottom of the editor. The UI's only floating cards are toasts:
 * ongoing state (missing samples, imports, an update, …) is an item here, its details in a
 * popover above it. The app's items come first, the extensions' (`shell.root` slot
 * `status-bar`) at the far end.
 */
export function StatusBar() {
  return (
    <footer className="eth-status-bar" data-slot="status-bar" data-testid="status-bar">
      <MissingPluginsStatus />
      <MissingMediaStatus />
      <ImportStatus />
      <div className="eth-status-bar__spacer" />
      <ExtensionRoots shell="editor" slot="status-bar" />
    </footer>
  );
}
