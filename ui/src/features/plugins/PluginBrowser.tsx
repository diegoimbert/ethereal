import { useCallback, useEffect, useState } from "react";
import type { PluginDescriptor } from "@/generated";
import { resolveSelectedTrack } from "@/features/devices/selectedTrack";
import { Button } from "@/kit";
import { devicesOfTrack, useProjectStore, useSelectedTrackId } from "@/state";
import { cmd, newId, useTransport, useTransportEvent } from "@/transport";
import { canInsert, filterPlugins, insertCommand } from "./filter";
import { useOptionalTransport, usePluginEvents, usePluginStore } from "./pluginStore";

const CATEGORY_LABEL: Record<PluginDescriptor["category"], string> = {
  Instrument: "Instrument",
  AudioEffect: "Effect",
  NoteEffect: "Note FX",
};

function errorText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/**
 * Scanned CLAP plugins (desktop only): search, rescan, and click (or Enter) to insert on
 * the selected track's device chain. Instruments go to the start of MIDI tracks' chains.
 */
export function PluginList() {
  usePluginEvents();
  const transport = useTransport();
  const plugins = usePluginStore((s) => s.plugins);
  const scan = usePluginStore((s) => s.scan);
  const failed = usePluginStore((s) => s.failed);
  const setPlugins = usePluginStore((s) => s.setPlugins);
  const setScan = usePluginStore((s) => s.setScan);
  const [query, setQuery] = useState("");
  const [message, setMessage] = useState<string | null>(null);
  const selectedId = useSelectedTrackId();
  const track = useProjectStore((s) => (s.project ? resolveSelectedTrack(s.project, selectedId) : undefined));

  const refresh = useCallback(() => {
    transport
      .send(cmd("Plugin", { type: "List" }))
      .then((reply) => {
        if (reply.type === "Plugins") setPlugins(reply.plugins);
      })
      .catch((e: unknown) => setMessage(`Could not list plugins: ${errorText(e)}`));
  }, [transport, setPlugins]);

  useEffect(refresh, [refresh]);
  useTransportEvent((e) => {
    if (e.type === "Plugin" && e.event.type === "ScanFinished") refresh();
  });

  const rescan = () => {
    setMessage(null);
    setScan({ done: 0, total: 0, current: null });
    transport.send(cmd("Plugin", { type: "Rescan" })).catch((e: unknown) => {
      setScan(null);
      setMessage(`Rescan failed: ${errorText(e)}`);
    });
  };

  const insert = (plugin: PluginDescriptor) => {
    const project = useProjectStore.getState().project;
    if (!project || !track) return;
    setMessage(null);
    const command = insertCommand(plugin, track, devicesOfTrack(project, track.id), newId());
    transport
      .send(command)
      .then(() => setMessage(`Inserted ${plugin.name} on ${track.name}`))
      .catch((e: unknown) => setMessage(`Could not insert ${plugin.name}: ${errorText(e)}`));
  };

  const shown = plugins ? filterPlugins(plugins, query) : [];

  return (
    <div className="eth-plugins" data-feature="plugins">
      <div className="eth-plugins__toolbar">
        <input
          className="eth-plugins__search"
          type="search"
          placeholder="Search plugins"
          aria-label="Search plugins"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <Button size="sm" onClick={rescan} disabled={scan !== null} title="Scan the plugin folders again">
          {scan ? "Scanning…" : "Rescan"}
        </Button>
      </div>
      {scan && (
        <div className="eth-plugins__status" role="status">
          {scan.total > 0 ? `Scanning ${scan.done}/${scan.total}` : "Scanning…"}
          {scan.current && <span className="eth-plugins__current"> {scan.current}</span>}
        </div>
      )}
      {!scan && failed.length > 0 && (
        <div className="eth-plugins__status" title={failed.map((f) => `${f.path}: ${f.message}`).join("\n")}>
          {failed.length} plugin{failed.length === 1 ? "" : "s"} failed to load
        </div>
      )}
      <div className="eth-plugins__target">{track ? `Insert on: ${track.name}` : "No track selected"}</div>
      <ul className="eth-plugins__list" aria-label="Plugins">
        {plugins === null && <li className="eth-plugins__empty">Loading…</li>}
        {plugins?.length === 0 && <li className="eth-plugins__empty">No plugins found. Install CLAP plugins and rescan.</li>}
        {plugins && plugins.length > 0 && shown.length === 0 && <li className="eth-plugins__empty">No match.</li>}
        {shown.map((p) => {
          const ok = canInsert(p, track);
          return (
            <li key={p.id}>
              <button
                type="button"
                className="eth-plugins__item"
                disabled={!ok}
                title={
                  ok
                    ? `${p.name} by ${p.vendor || "unknown vendor"}: click to insert on ${track?.name ?? "the track"}`
                    : `${CATEGORY_LABEL[p.category]}s can only be inserted on MIDI tracks`
                }
                onClick={() => insert(p)}
              >
                <span className="eth-plugins__name">{p.name}</span>
                <span className="eth-plugins__vendor">{p.vendor}</span>
                <span className="eth-plugins__category">{CATEGORY_LABEL[p.category]}</span>
              </button>
            </li>
          );
        })}
      </ul>
      {message && (
        <div className="eth-plugins__message" role="status">
          {message}
        </div>
      )}
    </div>
  );
}

/** The "plugins" sidebar tab: the plugin list on desktop, a notice elsewhere. */
export function PluginBrowserView() {
  const transport = useOptionalTransport();
  if (transport?.kind !== "tauri") {
    return (
      <div className="eth-plugins eth-plugins--unavailable" data-feature="plugins">
        <p>Plugins are available in the desktop app.</p>
      </div>
    );
  }
  return <PluginList />;
}
