import "./remote.css";
import type { FormEvent } from "react";
import { Trans, useT } from "@/i18n";
import { Button, TextInput } from "@/kit";
import { useAudioSettings } from "@/features/audio-settings/store";
import { useEngineServer } from "./engineServer";
import { useEngineServerShown } from "./visibility";
import { normalizeServerUrl } from "./url";

/**
 * Settings > Advanced > Engine server (base-115; base-114 naming): run
 * this window on an engine server (`ether-server`) on another machine instead of the built-in
 * engine. While connected the whole UI runs against it (the local engine resumes on
 * disconnect); the top bar shows "● <server>". Hidden until turned on (#717, `visibility.ts`):
 * `ether-server` isn't in any release.
 */
/** Command names, not translated. */
const SERVER_COMMAND = "ether-server";
const DEV_COMMAND = "just dev-server";

export function RemoteEngineSettings() {
  const t = useT();
  const s = useEngineServer();
  const shown = useEngineServerShown((v) => v.shown);
  if (!s.available || (!shown && !s.remote)) return null;
  const submit = (e?: FormEvent) => {
    e?.preventDefault();
    // Connected: the whole window switched engines; the settings close (like the old dialog).
    void s.connect().then((ok) => ok && useAudioSettings.getState().close());
  };
  return (
    <section className="eth-remote__form" aria-label={t("remote.title")} data-testid="settings-engine-server">
      <h3 className="eth-remote__heading">{t("remote.title")}</h3>
      {s.remote ? (
        <div className="eth-remote__connected" data-testid="remote-connected">
          <p className="eth-remote__status">
            <Trans
              k="remote.connected_to"
              values={{ name: <strong className="eth-user-text">{s.remote.info?.name}</strong>, url: <span className="eth-user-text">{s.remote.url}</span> }}
            />
          </p>
          <p className="eth-remote__hint">{t("remote.version", { version: s.remote.info?.app_version ?? "", instance: s.remote.info?.instance ?? "" })}</p>
          <div>
            <Button size="sm" tone="danger" onClick={s.disconnect}>
              {t("remote.disconnect")}
            </Button>
          </div>
        </div>
      ) : (
        <form className="eth-remote__form" onSubmit={submit}>
          <p className="eth-remote__hint">
            <Trans k="remote.hint" values={{ server: <code>{SERVER_COMMAND}</code> }} />{" "}
            <Trans k="remote.hint_dev" values={{ command: <code>{DEV_COMMAND}</code> }} />
          </p>
          <div className="eth-remote__row">
            <label className="eth-remote__field">
              <span>{t("remote.address")}</span>
              <TextInput
                size="sm"
                aria-label={t("remote.server_address")}
                placeholder={t("remote.address_placeholder")}
                value={s.url}
                invalid={!!s.error && !normalizeServerUrl(s.url)}
                onChange={(e) => s.setUrl(e.target.value)}
              />
            </label>
            <label className="eth-remote__field">
              <span>{t("remote.token")}</span>
              <TextInput
                size="sm"
                aria-label={t("remote.token")}
                type="password"
                autoComplete="off"
                value={s.token}
                onChange={(e) => s.setToken(e.target.value)}
              />
            </label>
          </div>
          {s.error && (
            <p className="eth-remote__form-error" role="alert">
              {s.error}
            </p>
          )}
          <div>
            <Button size="sm" type="submit" disabled={s.busy}>
              {s.busy ? t("remote.connecting") : t("remote.connect")}
            </Button>
          </div>
        </form>
      )}
    </section>
  );
}
