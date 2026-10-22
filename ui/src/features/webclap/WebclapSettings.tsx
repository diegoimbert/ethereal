/**
 * Settings > Plugins in the web version: the WebCLAP plugins installed in this browser
 * (install from an address or a file, remove). Native plugins (clap, vst3, au) run in the
 * desktop app.
 */
import { useRef, useState, type FormEvent } from "react";
import { FileUp, Trash2 } from "lucide-react";
import { Button, IconButton, TextInput, useConfirm } from "@/kit";
import { notify } from "@/features/notifications/store";
import { useT } from "@/i18n";
import type { WebclapApi } from "./api";
import { useWebclapModules } from "./hooks";
import "./webclap.css";

/** What the file picker accepts. */
const ACCEPT = ".wasm,.wclap,.gz,.tgz,application/wasm,application/gzip";

export function WebclapSettings({ api }: { api: WebclapApi }) {
  const t = useT();
  const modules = useWebclapModules(api);
  const [url, setUrl] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const file = useRef<HTMLInputElement>(null);

  const install = (job: () => Promise<{ name: string }>) => {
    setBusy(true);
    setError(null);
    job()
      .then((m) => {
        setUrl("");
        notify("Info", t("webclap.settings.installed", { name: m.name }));
      })
      .catch((e: unknown) => setError(t("webclap.settings.failed", { error: e instanceof Error ? e.message : String(e) })))
      .finally(() => setBusy(false));
  };
  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    if (url.trim()) install(() => api.installUrl(url.trim()));
  };
  const remove = (id: string, name: string) =>
    useConfirm.getState().ask({
      title: t("webclap.settings.remove", { name }),
      body: t("webclap.settings.remove_body"),
      action: t("webclap.settings.remove_action"),
      danger: true,
      run: () => {
        void api.remove(id).then(
          () => notify("Info", t("webclap.settings.removed", { name })),
          (e: unknown) => setError(e instanceof Error ? e.message : String(e)),
        );
      },
    });

  return (
    <div className="eth-webclap" data-feature="webclap-settings">
      <p className="eth-webclap__note">{t("webclap.settings.intro")}</p>
      <form className="eth-webclap__install" onSubmit={onSubmit}>
        <TextInput
          type="url"
          className="eth-webclap__url"
          aria-label={t("webclap.settings.url_label")}
          placeholder={t("webclap.settings.url_placeholder")}
          value={url}
          disabled={busy}
          onChange={(e) => setUrl(e.target.value)}
        />
        <Button type="submit" size="sm" disabled={busy || !url.trim()}>
          {busy ? t("webclap.settings.installing") : t("webclap.settings.install_url")}
        </Button>
        <Button type="button" size="sm" tone="ghost" disabled={busy} onClick={() => file.current?.click()}>
          <FileUp aria-hidden /> {t("webclap.settings.install_file")}
        </Button>
        <input
          ref={file}
          type="file"
          hidden
          accept={ACCEPT}
          data-testid="webclap-file"
          onChange={(e) => {
            const f = e.target.files?.[0];
            e.target.value = "";
            if (f) install(() => api.installFile(f));
          }}
        />
      </form>
      <p className="eth-webclap__note">{t("webclap.settings.trust")}</p>
      {error && (
        <p className="eth-webclap__error" role="alert">
          {error}
        </p>
      )}
      <section className="eth-webclap__section" aria-label={t("webclap.settings.installed_list")}>
        <h3 className="eth-webclap__heading">{t("webclap.settings.installed_list")}</h3>
        {modules.length === 0 ? (
          <p className="eth-webclap__note">{t("webclap.settings.empty")}</p>
        ) : (
          <ul className="eth-webclap__list">
            {modules.map((m) => (
              <li key={m.id} className="eth-webclap__module" data-webclap-module={m.id}>
                <div className="eth-webclap__module-head">
                  <span className="eth-webclap__name eth-user-text">{m.name}</span>
                  <span className="eth-webclap__dim">{t("webclap.settings.plugins_count", { count: m.plugins.length })}</span>
                  <IconButton
                    size="sm"
                    tone="ghost"
                    label={t("webclap.settings.remove", { name: m.name })}
                    icon={<Trash2 />}
                    onClick={() => remove(m.id, m.name)}
                  />
                </div>
                <span className="eth-webclap__source eth-user-text" title={m.source}>
                  {m.source}
                </span>
                <ul className="eth-webclap__plugins">
                  {m.plugins.map((p) => (
                    <li key={p.id} className="eth-user-text">
                      {p.name}
                      {p.vendor && <span className="eth-webclap__dim"> · {p.vendor}</span>}
                    </li>
                  ))}
                </ul>
                {m.failed.length > 0 && (
                  <span className="eth-webclap__error" title={m.failed.map((f) => `${f.id}: ${f.error}`).join("\n")}>
                    {t("webclap.settings.failed_plugins", { count: m.failed.length })}
                  </span>
                )}
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}
