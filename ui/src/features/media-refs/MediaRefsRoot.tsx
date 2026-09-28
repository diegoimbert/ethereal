import { AlertTriangle } from "lucide-react";
import { useContext, useEffect } from "react";
import type { MediaId } from "@/generated";
import { Button } from "@/kit";
import { useProjectStore } from "@/state";
import { TransportContext, type EngineTransport } from "@/transport";
import { RelinkDialog } from "./RelinkDialog";
import { applyMediaRefEvent, openRelink, refreshMissing, useMediaMissing, useMediaRefs } from "./store";

/** A clip's "sample missing" marker (in its title bar). */
export function MissingClipBadge({ media }: { media: MediaId | null | undefined }) {
  const missing = useMediaMissing(media);
  if (!missing) return null;
  return (
    <span className="eth-media-refs__clip-badge" data-testid="clip-missing" title="Sample missing: right-click to relink">
      <AlertTriangle aria-hidden />
      Missing
    </span>
  );
}

/** Floating notice while samples are missing: opens the Relink dialog. */
export function MissingMediaNotice() {
  const count = useMediaRefs((s) => s.missing.size);
  const dialogOpen = useMediaRefs((s) => s.dialog !== null);
  if (count === 0 || dialogOpen) return null;
  return (
    <div className="eth-media-refs__notice" role="status" data-testid="missing-media-notice">
      <AlertTriangle className="eth-media-refs__notice-icon" aria-hidden />
      <span>{count === 1 ? "1 sample is missing" : `${count} samples are missing`}</span>
      <Button size="sm" onClick={() => openRelink()}>
        Relink…
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
