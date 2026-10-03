import "./remote.css";
import type { FormEvent } from "react";
import { Button, TextInput } from "@/kit";
import { useEngineServer } from "./engineServer";
import { normalizeServerUrl } from "./url";

/**
 * Settings > Advanced > Engine server (base-115, docs/SHARING.md §8.6; base-114 naming): run
 * this window on an engine server (`ether-server`) on another machine instead of the built-in
 * engine. While connected the whole UI runs against it (the local engine resumes on
 * disconnect); the top bar shows "● <server>".
 */
export function RemoteEngineSettings() {
  const s = useEngineServer();
  if (!s.available) return null;
  const submit = (e?: FormEvent) => {
    e?.preventDefault();
    void s.connect();
  };
  return (
    <section className="eth-remote__form" aria-label="Engine server" data-testid="settings-engine-server">
      <h3 className="eth-remote__heading">Engine server</h3>
      {s.remote ? (
        <div className="eth-remote__connected" data-testid="remote-connected">
          <p className="eth-remote__status">
            Connected to <strong>{s.remote.info?.name}</strong> ({s.remote.url})
          </p>
          <p className="eth-remote__hint">
            Ethereal {s.remote.info?.app_version} · instance {s.remote.info?.instance}
          </p>
          <div>
            <Button size="sm" tone="danger" onClick={s.disconnect}>
              Disconnect
            </Button>
          </div>
        </div>
      ) : (
        <form className="eth-remote__form" onSubmit={submit}>
          <p className="eth-remote__hint">
            Run this window on an engine server on another machine instead of the built-in engine. Start one with <code>ether-server</code> (or{" "}
            <code>just dev-server</code>); it prints its address and where to find the token.
          </p>
          <div className="eth-remote__row">
            <label className="eth-remote__field">
              <span>Address</span>
              <TextInput
                size="sm"
                aria-label="Server address"
                placeholder="ws://host:port"
                value={s.url}
                invalid={!!s.error && !normalizeServerUrl(s.url)}
                onChange={(e) => s.setUrl(e.target.value)}
              />
            </label>
            <label className="eth-remote__field">
              <span>Token</span>
              <TextInput size="sm" aria-label="Token" type="password" autoComplete="off" value={s.token} onChange={(e) => s.setToken(e.target.value)} />
            </label>
          </div>
          {s.error && (
            <p className="eth-remote__form-error" role="alert">
              {s.error}
            </p>
          )}
          <div>
            <Button size="sm" type="submit" disabled={s.busy}>
              {s.busy ? "Connecting…" : "Connect"}
            </Button>
          </div>
        </form>
      )}
    </section>
  );
}
