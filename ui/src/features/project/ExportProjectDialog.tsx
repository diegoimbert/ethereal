/**
 * Export project: the project menu's (and the project picker's) "Export project…" opens
 * this dialog, with the bundle's settings: whether to pack the media files (the project's
 * own and the samples it references in place) or the document alone.
 */
import { useState } from "react";
import { errorMessage, type EngineCommands } from "@/features/transport-bar/engine";
import { notify } from "@/features/notifications";
import { Button, Dialog, Toggle } from "@/kit";
import { exportProject } from "./actions";
import { useExportProject } from "./exportProjectStore";
import { useT } from "@/i18n";

export function ExportProjectDialog({ commands }: { commands: EngineCommands }) {
  const t = useT();
  const { target, includeMedia, hide, setIncludeMedia } = useExportProject();
  const [busy, setBusy] = useState(false);
  const transport = commands.transport;

  const run = async () => {
    if (!transport || !target) return;
    setBusy(true);
    try {
      if (await exportProject(transport, target.id, target.name, { includeMedia })) hide();
    } catch (e) {
      notify("Error", errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      open={target !== null}
      onClose={hide}
      title={t("project.export.title")}
      className="eth-project-export"
      footer={
        <>
          <Button tone="ghost" onClick={hide}>
            {t("project.actions.cancel")}
          </Button>
          <Button tone="accent" disabled={busy || !transport} onClick={() => void run()}>
            {busy ? t("project.export.exporting") : t("project.export.export")}
          </Button>
        </>
      }
    >
      <div className="eth-project-export__body">
        <Toggle checked={includeMedia} onChange={setIncludeMedia} label={t("project.export.pack_media")} aria-label={t("project.export.pack_media")} />
        <p className="eth-project-export__help">{includeMedia ? t("project.export.pack_hint") : t("project.export.document_hint")}</p>
      </div>
    </Dialog>
  );
}
