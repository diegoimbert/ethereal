import "./remote.css";
import { useEffect, useState } from "react";
import { Badge, Button, Dialog } from "@/kit";
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
  const sw = useTransportSwitch();
  const [open, setOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const remote = sw?.remote instanceof WsTransport ? sw.remote : null;

  // A dropped connection returns to the local engine.
  useEffect(() => {
    if (!remote || !sw) return;
    return remote.onClose((ev) => {
      setError(`Connection to ${remote.info?.name ?? remote.url} lost${ev.reason ? ` (${ev.reason})` : ""}.`);
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
          aria-label="Engine server"
          aria-haspopup="dialog"
          title={`Running on the engine server ${remote.url}`}
          data-testid="remote-button"
          onClick={() => setOpen(true)}
        >
          ● {remote.info?.name ?? "Engine server"}
        </Button>
      )}
      {error && (
        <button type="button" className="eth-remote__error" role="alert" title="Dismiss" onClick={() => setError(null)}>
          {error}
        </button>
      )}
      <UploadStatus />
      <Dialog
        open={open && !!remote}
        onClose={() => setOpen(false)}
        title="Engine server"
        footer={
          <>
            <Button
              onClick={() => {
                setOpen(false);
                openSettings("advanced");
              }}
            >
              Settings…
            </Button>
            <Button tone="danger" onClick={disconnect}>
              Disconnect
            </Button>
          </>
        }
      >
        {remote && (
          <div className="eth-remote__form" data-testid="remote-connected">
            <p>
              Connected to <strong>{remote.info?.name}</strong> ({remote.url})
            </p>
            <p className="eth-remote__hint">
              Ethereal {remote.info?.app_version} · instance {remote.info?.instance}
            </p>
          </div>
        )}
      </Dialog>
    </div>
  );
}

function UploadStatus() {
  const uploads = useUploads();
  if (uploads.length === 0) return null;
  return (
    <span className="eth-remote__uploads" role="status" aria-label="Uploads">
      {uploads.map((u) =>
        u.error ? (
          <button key={u.id} type="button" className="eth-remote__error" title="Dismiss" onClick={() => uploadStore.remove(u.id)}>
            {u.name}: {u.error}
          </button>
        ) : (
          <Badge key={u.id} tone="accent">
            ↑ {u.name} {Math.floor((100 * u.sent) / u.size)}%
          </Badge>
        ),
      )}
    </span>
  );
}
