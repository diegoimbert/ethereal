import { AlertTriangle } from "lucide-react";
import { useContext, useEffect } from "react";
import { openContextMenu } from "@/kit";
import { StatusButton } from "@/features/status-bar/StatusButton";
import { openResolver } from "@/features/import-resolver/store";
import { resolveLabel } from "@/features/import-resolver/menus";
import { useT } from "@/i18n";
import { useProjectStore } from "@/state";
import { TransportContext, type EngineTransport } from "@/transport";
import { MediaManagerDialog } from "./MediaManagerDialog";
import { RelinkDialog } from "./RelinkDialog";
import { applyMediaRefEvent, openRelink, refreshMissing, useMediaRefs } from "./store";

/** Status bar item while samples are missing: opens the Relink dialog. */
export function MissingMediaStatus() {
  const t = useT();
  const count = useMediaRefs((s) => s.missing.size);
  const dialogOpen = useMediaRefs((s) => s.dialog !== null);
  if (count === 0 || dialogOpen) return null;
  return (
    <StatusButton
      tone="warn"
      icon={<AlertTriangle aria-hidden />}
      title={t("media_refs.notice.relink")}
      data-testid="missing-media-notice"
      onClick={() => openRelink()}
      // Right-click "Resolve…" every missing item.
      onContextMenu={(e) => openContextMenu(e, [{ label: resolveLabel(), onSelect: () => void openResolver() }])}
    >
      {t("media_refs.notice.missing", { count })}
    </StatusButton>
  );
}

function Sync({ transport }: { transport: EngineTransport }) {
  const projectId = useProjectStore((s) => s.project?.id ?? null);
  useEffect(() => transport.onEvent(applyMediaRefEvent), [transport]);
  // A UI that connects to an engine with a project already open asks what is missing.
  useEffect(() => {
    if (projectId) void refreshMissing(transport);
  }, [transport, projectId]);
  return null;
}

/**
 * Mounted once by the app shell: mirrors missing media from the engine and shows the media
 * manager and the Relink dialog (the status bar shows what is missing). Renders nothing without an engine connection.
 */
export function MediaRefsRoot() {
  const transport = useContext(TransportContext)?.transport;
  if (!transport) return null;
  return (
    <>
      <Sync transport={transport} />
      <MediaManagerDialog transport={transport} />
      <RelinkDialog transport={transport} />
    </>
  );
}
