import { webclapOf } from "@/features/webclap/hooks";
import { WebclapSettings } from "@/features/webclap/WebclapSettings";
import { useCallback, useEffect, useState } from "react";
import { SettingsPlaceholder } from "@/features/audio-settings/SettingsPlaceholder";
import { useContributions } from "@/ext";
import { FolderPlus, X } from "lucide-react";
import type { PluginCommand, PluginFolders, PluginFormat } from "@/generated";
import { hasFolderPicker } from "@/features/media-refs/sources";
import { useT } from "@/i18n";
import { Badge, Button, IconButton, Select, Toggle, type SelectOption } from "@/kit";
import { cmd, commandErrorText, isCommandFailed, type EngineTransport } from "@/transport";
import { FailedPluginsDialog } from "./FailedPlugins";
import { FORMAT_LABEL } from "./filter";
import { usePluginEvents, usePluginStore } from "./pluginStore";

/** `Any` is a `null` format filter on the wire. */
type FormatChoice = PluginFormat | "Any";

const FORMATS: ReadonlyArray<PluginFormat> = ["Clap", "Vst3", "Vst2", "Au"];

function errorText(e: unknown): string {
  if (isCommandFailed(e)) return commandErrorText(e.error);
  return e instanceof Error ? e.message : String(e);
}

/**
 * Settings → Plugins: where plugins are looked for. The system folders of each format
 * (read-only, on/off), the user's folders (each limited to one format or any), and the
 * scan buttons. Adding, removing or re-filtering a folder rescans incrementally (only new
 * or changed plugins are loaded); Full rescan loads every plugin again. "Failed plugins"
 * lists the ones that couldn't be loaded, each retryable on its own. Desktop only: the
 * browser and remote UIs have no native plugins (or no local folder picker).
 */
/** The folders as last loaded (this session): a later visit shows them at once, then refreshes. */
let lastFolders: PluginFolders | null = null;
const rememberFolders = (f: PluginFolders) => {
  lastFolders = f;
};

export function PluginFoldersPanel({ transport }: { transport: EngineTransport }) {
  const t = useT();
  const webclap = webclapOf(transport);
  if (webclap) {
    // The web version: its WebCLAP plugins (native ones run in the desktop app).
    return (
      <div className="eth-plugin-folders">
        <WebclapSettings api={webclap} />
        <PluginActions place="folders-web" className="eth-plugin-folders__scan" />
      </div>
    );
  }
  if (transport.kind !== "tauri") {
    return (
      <div className="eth-plugin-folders">
        <p className="eth-plugin-folders__note">{t("plugins.folders.desktop_only")}</p>
        <PluginActions place="folders-web" className="eth-plugin-folders__scan" />
      </div>
    );
  }
  return <FoldersBody transport={transport} />;
}

function FoldersBody({ transport }: { transport: EngineTransport }) {
  const t = useT();
  usePluginEvents();
  const scan = usePluginStore((s) => s.scan);
  const setScan = usePluginStore((s) => s.setScan);
  const failed = usePluginStore((s) => s.failed);
  const setFailed = usePluginStore((s) => s.setFailed);
  const [folders, setFoldersState] = useState<PluginFolders | null>(() => lastFolders);
  const setFolders = useCallback((f: PluginFolders) => {
    rememberFolders(f);
    setFoldersState(f);
  }, []);
  const [showFailed, setShowFailed] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const picker = hasFolderPicker(transport) ? transport : null;

  /** Send a folder command; its reply is the new folder list. */
  const run = useCallback(
    async (command: PluginCommand, scans: boolean) => {
      try {
        if (scans) setScan({ done: 0, total: 0, current: null });
        const reply = await transport.send(cmd("Plugin", command));
        if (reply.type === "PluginFolders") setFolders(reply.folders);
        setError(null);
      } catch (e) {
        if (scans) setScan(null);
        setError(errorText(e));
      }
    },
    [transport, setScan, setFolders],
  );

  useEffect(() => {
    let live = true;
    transport
      .send(cmd("Plugin", { type: "ListFolders" }))
      .then((reply) => {
        if (live && reply.type === "PluginFolders") setFolders(reply.folders);
      })
      .catch((e: unknown) => {
        if (live) setError(errorText(e));
      });
    return () => {
      live = false;
    };
  }, [transport, setFolders]);

  // Failures are known before any scan this session (the engine remembers the last one's).
  useEffect(() => {
    let live = true;
    transport
      .send(cmd("Plugin", { type: "ListScanFailures" }))
      .then((reply) => {
        if (live && reply.type === "ScanFailures") setFailed(reply.failed);
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [transport, setFailed]);

  const add = async () => {
    const path = await picker?.pickFolder();
    if (path) await run({ type: "AddFolder", path, format: null }, true);
  };
  const rescan = (full: boolean) => {
    setError(null);
    setScan({ done: 0, total: 0, current: null });
    transport.send(cmd("Plugin", full ? { type: "Rescan", full: true } : { type: "Rescan" })).catch((e: unknown) => {
      setScan(null);
      setError(t("plugins.folders.rescan_failed", { error: errorText(e) }));
    });
  };

  // Loading: placeholder rows where the folders go (no jump when they come).
  if (!folders) return error ? <p className="eth-plugin-folders__note">{error}</p> : <SettingsPlaceholder rows={4} />;
  const failures = failed.filter((f) => f.path).length;
  const formatChoices: SelectOption<FormatChoice>[] = [
    { value: "Any", label: t("plugins.folders.any_format") },
    ...FORMATS.map((f) => ({ value: f, label: FORMAT_LABEL[f] })),
  ];

  return (
    <div className="eth-plugin-folders">
      {error && (
        <p className="eth-plugin-folders__error" role="alert">
          {error}
        </p>
      )}

      <section className="eth-plugin-folders__section" aria-label={t("plugins.folders.yours_label")}>
        <div className="eth-plugin-folders__head">
          <h3 className="eth-plugin-folders__heading">{t("plugins.folders.yours")}</h3>
          <Button
            size="sm"
            className="eth-plugin-folders__add"
            disabled={!picker || scan !== null}
            title={picker ? t("plugins.folders.add_hint") : t("plugins.folders.no_picker")}
            onClick={() => void add()}
          >
            <FolderPlus aria-hidden />
            {t("plugins.folders.add")}
          </Button>
        </div>
        {folders.folders.length === 0 ? (
          <p className="eth-plugin-folders__note">{t("plugins.folders.none")}</p>
        ) : (
          <ul className="eth-plugin-folders__list" aria-label={t("plugins.folders.yours")}>
            {folders.folders.map((f) => (
              <li key={f.path} className="eth-plugin-folders__row">
                <span className="eth-plugin-folders__path eth-user-text" title={f.path}>
                  {f.path}
                </span>
                <Select
                  size="sm"
                  aria-label={t("plugins.folders.formats_in", { path: f.path })}
                  value={f.format ?? "Any"}
                  options={formatChoices}
                  disabled={scan !== null}
                  onChange={(v) => void run({ type: "AddFolder", path: f.path, format: v === "Any" ? null : v }, true)}
                />
                <IconButton
                  size="sm"
                  tone="ghost"
                  label={t("plugins.folders.remove", { path: f.path })}
                  icon={<X />}
                  disabled={scan !== null}
                  onClick={() => void run({ type: "RemoveFolder", path: f.path }, true)}
                />
              </li>
            ))}
          </ul>
        )}
      </section>

      <section className="eth-plugin-folders__section" aria-label={t("plugins.folders.system_label")}>
        <div className="eth-plugin-folders__head">
          <h3 className="eth-plugin-folders__heading">{t("plugins.folders.system")}</h3>
          <Toggle
            size="sm"
            label={t("plugins.folders.scan")}
            checked={folders.include_defaults}
            disabled={scan !== null}
            onChange={(include) => void run({ type: "SetIncludeDefaults", include }, true)}
          />
        </div>
        <ul
          className="eth-plugin-folders__list eth-plugin-folders__list--defaults"
          aria-label={t("plugins.folders.system")}
          data-off={folders.include_defaults ? undefined : ""}
        >
          {folders.defaults.map((d) => (
            <li key={`${d.format}:${d.path}`} className="eth-plugin-folders__row">
              <span className="eth-plugin-folders__path eth-user-text" title={d.path}>
                {d.path}
              </span>
              {!d.exists && <span className="eth-plugin-folders__missing">{t("plugins.folders.not_found")}</span>}
              <Badge>{FORMAT_LABEL[d.format]}</Badge>
            </li>
          ))}
        </ul>
        {!folders.include_defaults && <p className="eth-plugin-folders__note">{t("plugins.folders.system_skipped")}</p>}
      </section>

      <div className="eth-plugin-folders__scan">
        <Button size="sm" onClick={() => rescan(false)} disabled={scan !== null} title={t("plugins.folders.rescan_hint")}>
          {scan ? t("plugins.scan.scanning") : t("plugins.folders.rescan")}
        </Button>
        <Button size="sm" variant="ghost" onClick={() => rescan(true)} disabled={scan !== null} title={t("plugins.folders.full_rescan_hint")}>
          {t("plugins.folders.full_rescan")}
        </Button>
        <PluginActions place="folders" />
        {failures > 0 && (
          <Button size="sm" variant="ghost" onClick={() => setShowFailed(true)} title={t("plugins.folders.failed_hint")}>
            {t("plugins.folders.failed", { count: failures })}
          </Button>
        )}
        <span className="eth-plugin-folders__status" role="status">
          {scan
            ? scan.total > 0
              ? t("plugins.scan.progress", { done: scan.done, total: scan.total })
              : t("plugins.scan.scanning")
            : failures > 0
              ? t("plugins.scan.failed_count", { count: failures })
              : ""}
        </span>
      </div>
      <FailedPluginsDialog transport={transport} open={showFailed} onClose={() => setShowFailed(false)} />
    </div>
  );
}

/** The extensions' plugin actions ("explore plugins"; `@/ext` "plugins.actions"), if any. */
function PluginActions({ place, className }: { place: "folders" | "folders-web"; className?: string }) {
  const actions = useContributions("plugins.actions");
  if (actions.length === 0) return null;
  const all = actions.map((Action, i) => <Action key={i} place={place} />);
  return className ? <div className={className}>{all}</div> : <>{all}</>;
}
