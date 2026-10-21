import { AlertTriangle, Upload } from "lucide-react";
import type { CSSProperties } from "react";
import { StatusButton, StatusText } from "@/features/status-bar/StatusButton";
import { useT } from "@/i18n";
import { uploadStore, useUploads } from "./upload";

/**
 * The status bar's uploads to an engine server: each file's name and progress, a failed one
 * its error (a click dismisses it). Nothing while none is.
 */
export function RemoteUploadsStatus() {
  const t = useT();
  const uploads = useUploads();
  if (uploads.length === 0) return null;
  return (
    <span className="eth-status-bar__group" role="group" aria-label={t("remote.uploads")}>
      {uploads.map((u) =>
        u.error ? (
          <StatusButton key={u.id} tone="danger" icon={<AlertTriangle aria-hidden />} title={t("common.dismiss")} onClick={() => uploadStore.remove(u.id)}>
            <span className="eth-user-text">{u.name}</span>: {u.error}
          </StatusButton>
        ) : (
          <StatusText key={u.id} icon={<Upload aria-hidden />}>
            <span className="eth-user-text">{u.name}</span>
            <span className="eth-status-bar__progress" aria-hidden>
              <span className="eth-status-bar__progress-fill" style={{ "--eth-status-progress": u.sent / Math.max(1, u.size) } as CSSProperties} />
            </span>
            {Math.floor((100 * u.sent) / Math.max(1, u.size))}%
          </StatusText>
        ),
      )}
    </span>
  );
}
