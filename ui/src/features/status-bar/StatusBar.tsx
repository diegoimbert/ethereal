import { Settings } from "lucide-react";
import { ExtensionRoots } from "@/app/ExtensionRoots";
import { openSettings } from "@/features/audio-settings/store";
import { useT } from "@/i18n";
import { StatusButton } from "./StatusButton";
import { ImportStatus } from "@/features/import/ImportStatus";
import { MissingPluginsStatus } from "@/features/import-resolver/MissingItems";
import { MissingMediaStatus } from "@/features/media-refs/MediaRefsRoot";
import { SafeModeStatus } from "@/features/project/SafeModeStatus";
import { RemoteUploadsStatus } from "@/features/remote/UploadsStatus";
import "./statusBar.css";

/**
 * The thin bar along the bottom of the editor. The UI's only floating cards are toasts:
 * ongoing state (safe mode, missing samples, imports, an update, …) is an item here, its details in a
 * popover above it. The app's items come first, the extensions' (`shell.root` slot
 * `status-bar`) at the far end, then the settings button.
 */
export function StatusBar() {
  const t = useT();
  return (
    <footer className="eth-status-bar" data-slot="status-bar" data-testid="status-bar">
      <SafeModeStatus />
      <MissingPluginsStatus />
      <MissingMediaStatus />
      <ImportStatus />
      <RemoteUploadsStatus />
      <div className="eth-status-bar__spacer" />
      <ExtensionRoots shell="editor" slot="status-bar" />
      {/* The settings, at the far right. */}
      <StatusButton icon={<Settings aria-hidden />} aria-label={t("settings.title")} data-tour="settings-button" onClick={() => openSettings()} />
    </footer>
  );
}
