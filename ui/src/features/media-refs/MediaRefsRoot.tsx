import { AlertTriangle } from "lucide-react";
import { useContext, useEffect } from "react";
import { Button, openContextMenu } from "@/kit";
import { openResolver } from "@/features/import-resolver/store";
import { resolveLabel } from "@/features/import-resolver/menus";
import { useT } from "@/i18n";
import { useProjectStore } from "@/state";
import { TransportContext, type EngineTransport } from "@/transport";
import { RelinkDialog } from "./RelinkDialog";
import { applyMediaRefEvent, openRelink, refreshMissing, useMediaRefs } from "./store";

/** Floating notice while samples are missing: opens the Relink dialog. */
export function MissingMediaNotice() {
  const t = useT();
  const count = useMediaRefs((s) => s.missing.size);
  const dialogOpen = useMediaRefs((s) => s.dialog !== null);
  if (count === 0 || dialogOpen) return null;
  return (
    <div
      className="eth-media-refs__notice"
      role="status"
      data-testid="missing-media-notice"
      // Right-click "Resolve…" every missing item.
      onContextMenu={(e) => openContextMenu(e, [{ label: resolveLabel(), onSelect: () => void openResolver() }])}
    >
      <AlertTriangle className="eth-media-refs__notice-icon" aria-hidden />
      <span>{t("media_refs.notice.missing", { count })}</span>
      <Button size="sm" onClick={() => openRelink()}>
        {t("media_refs.notice.relink")}
      </Button>
    </div>
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
 * Mounted once by the app shell: mirrors missing media from the engine, shows the missing
 * notice and the Relink dialog. Renders nothing without an engine connection.
 */
export function MediaRefsRoot() {
  const transport = useContext(TransportContext)?.transport;
  if (!transport) return null;
  return (
    <>
      <Sync transport={transport} />
      <MissingMediaNotice />
      <RelinkDialog transport={transport} />
    </>
  );
}
