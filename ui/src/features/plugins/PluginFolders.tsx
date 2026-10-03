import { useCallback, useEffect, useState } from "react";
import { FolderPlus, X } from "lucide-react";
import type { PluginCommand, PluginFolders, PluginFormat } from "@/generated";
import { hasFolderPicker } from "@/features/media-refs/sources";
import { Badge, Button, IconButton, Select, Toggle, type SelectOption } from "@/kit";
import { cmd, isCommandFailed, type EngineTransport } from "@/transport";
import { FORMAT_LABEL } from "./filter";
import { usePluginEvents, usePluginStore } from "./pluginStore";

/** `Any` is a `null` format filter on the wire. */
type FormatChoice = PluginFormat | "Any";

const FORMAT_CHOICES: SelectOption<FormatChoice>[] = [
  { value: "Any", label: "Any format" },
  { value: "Clap", label: FORMAT_LABEL.Clap },
  { value: "Vst3", label: FORMAT_LABEL.Vst3 },
  { value: "Au", label: FORMAT_LABEL.Au },
];

function errorText(e: unknown): string {
  if (isCommandFailed(e)) return e.error.message || e.error.code;
  return e instanceof Error ? e.message : String(e);
}

/**
 * Settings → Plugins: where plugins are looked for. The system folders of each format
 * (read-only, on/off), the user's folders (each limited to one format or any), and the
 * scan buttons. Adding, removing or re-filtering a folder rescans incrementally (only new
 * or changed plugins are loaded); Full rescan loads every plugin again. Desktop only: the
 * browser and remote UIs have no native plugins (or no local folder picker).
 */
export function PluginFoldersPanel({ transport }: { transport: EngineTransport }) {
  if (transport.kind !== "tauri") {
    return (
      <p className="eth-plugin-folders__note">
        Plugins (CLAP, VST3, AU) are loaded by the desktop app; their folders are set there.
      </p>
    );
  }
  return <FoldersBody transport={transport} />;
}

function FoldersBody({ transport }: { transport: EngineTransport }) {
  usePluginEvents();
  const scan = usePluginStore((s) => s.scan);
  const setScan = usePluginStore((s) => s.setScan);
  const failed = usePluginStore((s) => s.failed);
  const [folders, setFolders] = useState<PluginFolders | null>(null);
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
    [transport, setScan],
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
  }, [transport]);

  const add = async () => {
    const path = await picker?.pickFolder();
    if (path) await run({ type: "AddFolder", path, format: null }, true);
  };
  const rescan = (full: boolean) => {
    setError(null);
    setScan({ done: 0, total: 0, current: null });
    transport.send(cmd("Plugin", full ? { type: "Rescan", full: true } : { type: "Rescan" })).catch((e: unknown) => {
      setScan(null);
      setError(`Rescan failed: ${errorText(e)}`);
    });
  };

  if (!folders) return <p className="eth-plugin-folders__note">{error ?? "Loading…"}</p>;
  const failures = failed.filter((f) => f.path).length;

  return (
    <div className="eth-plugin-folders">
      {error && (
        <p className="eth-plugin-folders__error" role="alert">
          {error}
        </p>
      )}

      <section className="eth-plugin-folders__section" aria-label="Your plugin folders">
        <div className="eth-plugin-folders__head">
          <h3 className="eth-plugin-folders__heading">Your folders</h3>
          <Button
            size="sm"
            className="eth-plugin-folders__add"
            disabled={!picker || scan !== null}
            title={picker ? "Add a folder to scan for plugins" : "No folder picker here"}
            onClick={() => void add()}
          >
            <FolderPlus aria-hidden />
            Add folder…
          </Button>
        </div>
        {folders.folders.length === 0 ? (
          <p className="eth-plugin-folders__note">No folders added. Add one to scan plugins installed outside the system folders.</p>
        ) : (
          <ul className="eth-plugin-folders__list" aria-label="Your folders">
            {folders.folders.map((f) => (
              <li key={f.path} className="eth-plugin-folders__row">
                <span className="eth-plugin-folders__path" title={f.path}>
                  {f.path}
                </span>
                <Select
                  size="sm"
                  aria-label={`Formats in ${f.path}`}
                  value={f.format ?? "Any"}
                  options={FORMAT_CHOICES}
                  disabled={scan !== null}
                  onChange={(v) => void run({ type: "AddFolder", path: f.path, format: v === "Any" ? null : v }, true)}
                />
                <IconButton
                  size="sm"
                  tone="ghost"
                  label={`Remove ${f.path}`}
                  icon={<X />}
                  disabled={scan !== null}
                  onClick={() => void run({ type: "RemoveFolder", path: f.path }, true)}
                />
              </li>
            ))}
          </ul>
        )}
      </section>

      <section className="eth-plugin-folders__section" aria-label="System plugin folders">
        <div className="eth-plugin-folders__head">
          <h3 className="eth-plugin-folders__heading">System folders</h3>
          <Toggle
            size="sm"
            label="Scan"
            checked={folders.include_defaults}
            disabled={scan !== null}
            onChange={(include) => void run({ type: "SetIncludeDefaults", include }, true)}
          />
        </div>
        <ul
          className="eth-plugin-folders__list eth-plugin-folders__list--defaults"
          aria-label="System folders"
          data-off={folders.include_defaults ? undefined : ""}
        >
          {folders.defaults.map((d) => (
            <li key={`${d.format}:${d.path}`} className="eth-plugin-folders__row">
              <span className="eth-plugin-folders__path" title={d.path}>
                {d.path}
              </span>
              {!d.exists && <span className="eth-plugin-folders__missing">not found</span>}
              <Badge>{FORMAT_LABEL[d.format]}</Badge>
            </li>
          ))}
        </ul>
        {!folders.include_defaults && (
          <p className="eth-plugin-folders__note">System folders are skipped (and, on macOS, Audio Units).</p>
        )}
      </section>

      <div className="eth-plugin-folders__scan">
        <Button size="sm" onClick={() => rescan(false)} disabled={scan !== null} title="Scan new or changed plugins">
          {scan ? "Scanning…" : "Rescan"}
        </Button>
        <Button
          size="sm"
          variant="ghost"
          onClick={() => rescan(true)}
          disabled={scan !== null}
          title="Load every plugin again, including ones that failed before"
        >
          Full rescan
        </Button>
        <span className="eth-plugin-folders__status" role="status">
          {scan
            ? scan.total > 0
              ? `Scanning ${scan.done}/${scan.total}`
              : "Scanning…"
            : failures > 0
              ? `${failures} plugin${failures === 1 ? "" : "s"} failed to load (Full rescan retries them)`
              : ""}
        </span>
      </div>
    </div>
  );
}
