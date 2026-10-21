import "./remote.css";
import { useEffect, useState } from "react";
import { t as tNow, Trans, useT } from "@/i18n";
import { Button, Dialog } from "@/kit";
import { notify } from "@/features/notifications";
import { useTransportSwitch } from "@/transport/createDefaultTransport";
import { WsTransport } from "@/transport/ws/WsTransport";
import { openSettings } from "@/features/audio-settings/store";

/**
 * The engine server in the top bar (`data-slot="remote"`). The connect form lives in
 * Settings > Advanced > Engine server. Here: "● <server>"
 * while this window runs on an engine server (its dialog disconnects). A dropped connection
 * returns to the local engine and says why (a toast); the uploads are in the status bar
 * (`RemoteUploadsStatus`).
 */
export function ConnectDialog() {
  const t = useT();
  const sw = useTransportSwitch();
  const [open, setOpen] = useState(false);
  const remote = sw?.remote instanceof WsTransport ? sw.remote : null;

  // A dropped connection returns to the local engine.
  useEffect(() => {
    if (!remote || !sw) return;
    return remote.onClose((ev) => {
      const server = remote.info?.name ?? remote.url;
      notify("Warning", ev.reason ? tNow("remote.lost_reason", { server, reason: ev.reason }) : tNow("remote.lost", { server }));
      sw.switchToLocal();
    });
  }, [remote, sw]);
  if (!sw) return null;

  const disconnect = () => {
    sw.switchToLocal();
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
