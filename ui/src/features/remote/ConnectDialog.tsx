import "./remote.css";
import { useEffect, useState } from "react";
import { t as tNow, Trans, useT } from "@/i18n";
import { Badge, Button, Dialog, DismissibleError } from "@/kit";
import { useTransportSwitch } from "@/transport/createDefaultTransport";
import { WsTransport } from "@/transport/ws/WsTransport";
import { openSettings } from "@/features/audio-settings/store";
import { uploadStore, useUploads } from "./upload";

/**
 * The engine server in the top bar (`data-slot="remote"`). The connect form lives in
 * Settings > Advanced > Engine server (base-115, docs/SHARING.md §8.6). Here: "● <server>"
 * while this window runs on an engine server (its dialog disconnects), why a connection was
 * lost, and the upload progress. A dropped connection returns to the local engine.
 */
export function ConnectDialog() {
  const t = useT();
  const sw = useTransportSwitch();
  const [open, setOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const remote = sw?.remote instanceof WsTransport ? sw.remote : null;

  // A dropped connection returns to the local engine.
  useEffect(() => {
    if (!remote || !sw) return;
    return remote.onClose((ev) => {
      const server = remote.info?.name ?? remote.url;
      setError(ev.reason ? tNow("remote.lost_reason", { server, reason: ev.reason }) : tNow("remote.lost", { server }));
      sw.switchToLocal();
    });
  }, [remote, sw]);
  // A new connection clears the last loss.
  const [seen, setSeen] = useState(remote);
  if (seen !== remote) {
    setSeen(remote);
    if (remote) setError(null);
  }

  if (!sw) return null;

  const disconnect = () => {
    sw.switchToLocal();
    setError(null);
    setOpen(false);
  };

  return (
    <div className="eth-remote" data-feature="remote">
      {remote && (
        <Button
          size="sm"
          active
          aria-label={t("remote.title")}
          aria-haspopup="dialog"
          title={t("remote.running_on", { url: remote.url })}
          data-testid="remote-button"
          onClick={() => setOpen(true)}
        >
          ● {remote.info ? <span className="eth-user-text">{remote.info.name}</span> : t("remote.title")}
        </Button>
      )}
      {error && (
        <DismissibleError className="eth-remote__error" onDismiss={() => setError(null)}>
          {error}
        </DismissibleError>
      )}
      <UploadStatus />
      <Dialog
        open={open && !!remote}
        onClose={() => setOpen(false)}
        title={t("remote.title")}
        footer={
          <>
            <Button
              onClick={() => {
                setOpen(false);
                openSettings("advanced");
              }}
            >
              {t("remote.settings")}
            </Button>
            <Button tone="danger" onClick={disconnect}>
              {t("remote.disconnect")}
            </Button>
          </>
        }
      >
        {remote && (
          <div className="eth-remote__form" data-testid="remote-connected">
            <p>
              <Trans
                k="remote.connected_to"
                values={{ name: <strong className="eth-user-text">{remote.info?.name}</strong>, url: <span className="eth-user-text">{remote.url}</span> }}
              />
            </p>
            <p className="eth-remote__hint">{t("remote.version", { version: remote.info?.app_version ?? "", instance: remote.info?.instance ?? "" })}</p>
          </div>
        )}
      </Dialog>
    </div>
  );
}

function UploadStatus() {
  const t = useT();
  const uploads = useUploads();
  if (uploads.length === 0) return null;
  return (
    <span className="eth-remote__uploads" role="status" aria-label={t("remote.uploads")}>
      {uploads.map((u) =>
        u.error ? (
          <button key={u.id} type="button" className="eth-remote__error" title={t("common.dismiss")} onClick={() => uploadStore.remove(u.id)}>
            <span className="eth-user-text">{u.name}</span>: {u.error}
          </button>
        ) : (
          <Badge key={u.id} tone="accent">
            ↑ <span className="eth-user-text">{u.name}</span> {Math.floor((100 * u.sent) / u.size)}%
          </Badge>
        ),
      )}
    </span>
  );
}
