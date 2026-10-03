// Connecting to an engine server (ether-server): the form state shared by Settings > Advanced
// (base-115 moved the form there, docs/SHARING.md §8.6). The top bar keeps an indicator while
// connected (`ConnectDialog`).
import { useState } from "react";
import type { HelloRejection } from "@/generated";
import { useTransportSwitch } from "@/transport/createDefaultTransport";
import { RemoteRejectedError, WsTransport } from "@/transport/ws/WsTransport";
import { normalizeServerUrl } from "./url";

/** Remembered server URL (never the token). */
export const URL_KEY = "eth-remote-url";

const REJECTION_TEXT: Record<HelloRejection, string> = {
  BadToken: "Wrong or missing token.",
  UnsupportedVersion: "This server runs an incompatible version of Ethereal.",
  Busy: "The server does not accept more clients right now.",
};

export function loadUrl(): string {
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

export function describeError(e: unknown): string {
  if (e instanceof RemoteRejectedError) return REJECTION_TEXT[e.reason] ?? e.message;
  return e instanceof Error ? e.message : String(e);
}

/** The engine server form: address, token, connect / disconnect. */
export function useEngineServer() {
  const sw = useTransportSwitch();
  const [url, setUrl] = useState(loadUrl);
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const remote = sw?.remote instanceof WsTransport ? sw.remote : null;

  /** Resolves whether the window now runs on the server. */
  const connect = async (): Promise<boolean> => {
    if (!sw) return false;
    const target = normalizeServerUrl(url);
    if (!target) {
      setError("Enter the engine server address, such as ws://studio.local:9000.");
      return false;
    }
    setBusy(true);
    setError(null);
    const t = new WsTransport(target, { token: token || null });
    try {
      await t.open();
      saveUrl(url.trim());
      setToken("");
      sw.switchToRemote(t);
      return true;
    } catch (err) {
      t.dispose();
      setError(describeError(err));
      return false;
    } finally {
      setBusy(false);
    }
  };

  const disconnect = () => {
    sw?.switchToLocal();
    setError(null);
  };

  return { available: sw !== null, remote, url, setUrl, token, setToken, busy, error, connect, disconnect };
}
