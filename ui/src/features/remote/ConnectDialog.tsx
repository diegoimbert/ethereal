import "./remote.css";
import { useEffect, useState, type FormEvent } from "react";
import type { HelloRejection } from "@/generated";
import { Badge, Button, Dialog, TextInput } from "@/kit";
import { useTransportSwitch } from "@/transport/createDefaultTransport";
import { RemoteRejectedError, WsTransport } from "@/transport/ws/WsTransport";
import { uploadStore, useUploads } from "./upload";
import { normalizeServerUrl } from "./url";

/** Remembered server URL (never the token). */
const URL_KEY = "eth-remote-url";

const REJECTION_TEXT: Record<HelloRejection, string> = {
  BadToken: "Wrong or missing token.",
  UnsupportedVersion: "This server runs an incompatible version of Ethereal.",
  Busy: "The server does not accept more clients right now.",
};

function loadUrl(): string {
  try {
    return localStorage.getItem(URL_KEY) ?? "";
  } catch {
    return "";
  }
}

function saveUrl(url: string) {
  try {
    localStorage.setItem(URL_KEY, url);
  } catch {
    // storage unavailable: nothing to remember
  }
}

function describeError(e: unknown): string {
  if (e instanceof RemoteRejectedError) return REJECTION_TEXT[e.reason] ?? e.message;
  return e instanceof Error ? e.message : String(e);
}

/**
 * Connect to an engine server (WebSocket URL + token): a top-bar "Engine server…" button
 * (base-114 naming; the app has no settings menu yet, so it stays in the top bar with a
 * tooltip; the Share redesign moves it to Settings > Advanced) that opens a dialog. While connected the whole UI runs against the remote engine (the local one is
 * stopped and resumes on disconnect). Also shows file upload progress.
 */
export function ConnectDialog() {
  const sw = useTransportSwitch();
  const [open, setOpen] = useState(false);
  const [url, setUrl] = useState(loadUrl);
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);
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

  if (!sw) return null;

  const connect = async (e?: FormEvent) => {
    e?.preventDefault();
    const target = normalizeServerUrl(url);
    if (!target) {
      setError("Enter the engine server address, such as ws://studio.local:9000.");
      return;
    }
    setBusy(true);
    setError(null);
    const t = new WsTransport(target, { token: token || null });
    try {
      await t.open();
      saveUrl(url.trim());
      sw.switchToRemote(t);
      setOpen(false);
    } catch (err) {
      t.dispose();
      setError(describeError(err));
    } finally {
      setBusy(false);
    }
  };

  const disconnect = () => {
    sw.switchToLocal();
    setError(null);
    setOpen(false);
  };

  const label = remote ? (remote.info?.name ?? "Engine server") : "Engine server…";
  return (
    <div className="eth-remote" data-feature="remote">
      <Button
        size="sm"
        active={!!remote}
        aria-label="Engine server"
        aria-haspopup="dialog"
        title={
          remote
            ? `Running on the engine server ${remote.url}`
            : "Run this window on an engine server (ether-server) on another machine instead of the built-in engine"
        }
        data-testid="remote-button"
        onClick={() => setOpen(true)}
      >
        {remote ? `● ${label}` : label}
      </Button>
      {error && !open && (
        <button type="button" className="eth-remote__error" role="alert" title="Dismiss" onClick={() => setError(null)}>
          {error}
        </button>
      )}
      <UploadStatus />
      <Dialog
        open={open}
        onClose={() => setOpen(false)}
        title="Engine server"
        footer={
          remote ? (
            <>
              <Button onClick={() => setOpen(false)}>Close</Button>
              <Button tone="danger" onClick={disconnect}>
                Disconnect
              </Button>
            </>
          ) : (
            <>
              <Button onClick={() => setOpen(false)}>Cancel</Button>
              <Button tone="accent" disabled={busy} onClick={() => void connect()}>
                {busy ? "Connecting…" : "Connect"}
              </Button>
            </>
          )
        }
      >
        {remote ? (
          <div className="eth-remote__form" data-testid="remote-connected">
            <p>
              Connected to <strong>{remote.info?.name}</strong> ({remote.url})
            </p>
            <p className="eth-remote__hint">
              Ethereal {remote.info?.app_version} · instance {remote.info?.instance}
            </p>
          </div>
        ) : (
          <form className="eth-remote__form" onSubmit={(e) => void connect(e)}>
            <label className="eth-remote__field">
              <span>Address</span>
              <TextInput
                aria-label="Server address"
                placeholder="ws://host:port"
                value={url}
                autoFocus
                invalid={!!error && !normalizeServerUrl(url)}
                onChange={(e) => setUrl(e.target.value)}
              />
            </label>
            <label className="eth-remote__field">
              <span>Token</span>
              <TextInput aria-label="Token" type="password" autoComplete="off" value={token} onChange={(e) => setToken(e.target.value)} />
            </label>
            <p className="eth-remote__hint">
              Start a server with <code>ether-server</code> (or <code>just dev-server</code>); it prints its address and where to find the token.
            </p>
            {error && (
              <p className="eth-remote__form-error" role="alert">
                {error}
              </p>
            )}
            {/* Enter submits. */}
            <button type="submit" hidden />
          </form>
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
