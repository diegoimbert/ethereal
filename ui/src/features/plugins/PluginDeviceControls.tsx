import { useState } from "react";
import type { Command, Device } from "@/generated";
import { Badge, Button } from "@/kit";
import { useProjectStore } from "@/state";
import { cmd, useTransport } from "@/transport";
import { FORMAT_LABEL, isInstalled } from "./filter";
import { usePluginEvents, usePluginStore, useScannedPlugins } from "./pluginStore";

export interface PluginDeviceControlsProps {
  device: Device;
}

/**
 * Plugin-specific controls in a device header (mounted by `DeviceView` for every device;
 * renders nothing for built-ins): editor window, sandbox toggle, and the safe-mode
 * placeholder, crashed and missing states (bypassed) with their Load / Reload action.
 * Works the same for every format.
 */
export function PluginDeviceControls({ device }: PluginDeviceControlsProps) {
  usePluginEvents();
  const transport = useTransport();
  const crash = usePluginStore((s) => s.crashed[device.id]);
  const editorOpen = usePluginStore((s) => !!s.editors[device.id]);
  const setEditorOpen = usePluginStore((s) => s.setEditorOpen);
  const clearCrash = usePluginStore((s) => s.clearCrash);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const plugins = useScannedPlugins(device.kind.type === "Plugin");
  // base-131: held as a placeholder (the project was opened without plugins).
  const held = useProjectStore((s) => s.safeMode.includes(device.id));

  if (device.kind.type !== "Plugin") return null;
  const { sandboxed, format, plugin_id } = device.kind.plugin;
  // Not in the scanned list: the host couldn't load it and the device is bypassed.
  const missing = plugins !== null && !isInstalled(plugins, { format, plugin_id });
  const name = device.name;

  const run = (command: Command, onOk?: () => void) => {
    setBusy(true);
    setError(null);
    transport
      .send(command)
      .then(() => onOk?.())
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)))
      .finally(() => setBusy(false));
  };

  const toggleEditor = () =>
    editorOpen
      ? run(cmd("Plugin", { type: "CloseEditor", device: device.id }), () => setEditorOpen(device.id, false))
      : run(cmd("Plugin", { type: "OpenEditor", device: device.id }), () => setEditorOpen(device.id, true));

  // Both re-instantiate the plugin (from its current / last saved state).
  const toggleSandbox = () =>
    run(cmd("Plugin", { type: "SetSandboxed", device: device.id, sandboxed: !sandboxed }), () =>
      clearCrash(device.id),
    );
  const reload = () => run(cmd("Plugin", { type: "Reload", device: device.id }), () => clearCrash(device.id));
  const load = () =>
    run(cmd("Plugin", { type: "Reload", device: device.id }), () => {
      const store = useProjectStore.getState();
      store.setSafeMode(store.safe, store.safeMode.filter((d) => d !== device.id));
    });

  return (
    <span className="eth-plugin-controls" data-plugin-controls={device.id}>
      {held ? (
        <>
          <Badge tone="warn" className="eth-plugin-controls__missing">
            <span role="status" title="Opened without plugins: bypassed, state kept. Load it, or Load plugins in the top bar.">
              safe mode · bypassed
            </span>
          </Badge>
          <Button size="sm" variant="ghost" disabled={busy} aria-label={`Load ${name}`} title="Load this plugin now" onClick={load}>
            ⟳
          </Button>
        </>
      ) : missing ? (
        <>
          <Badge tone="warn" className="eth-plugin-controls__missing">
            <span role="status" title={`${FORMAT_LABEL[format]} plugin ${plugin_id} is not installed. Rescan plugins, then reload.`}>
              missing · bypassed
            </span>
          </Badge>
          <Button size="sm" variant="ghost" disabled={busy} aria-label={`Reload ${name}`} title="Load the plugin again (after a rescan)" onClick={reload}>
            ⟳
          </Button>
        </>
      ) : crash !== undefined ? (
        <>
          <span className="eth-plugin-controls__crashed" role="status" title={crash}>
            crashed · bypassed
          </span>
          <Button size="sm" variant="ghost" disabled={busy} aria-label={`Reload ${name}`} title="Reload the plugin from its last saved state" onClick={reload}>
            ⟳
          </Button>
        </>
      ) : (
        <Button
          size="sm"
          variant="ghost"
          disabled={busy}
          active={editorOpen}
          aria-label={editorOpen ? `Close ${name} editor` : `Open ${name} editor`}
          title={editorOpen ? "Close the plugin window" : "Open the plugin window"}
          onClick={toggleEditor}
        >
          ▣
        </Button>
      )}
      <Button
        size="sm"
        variant="ghost"
        disabled={busy}
        active={sandboxed}
        aria-label={`Sandbox ${name}`}
        title={
          sandboxed
            ? "Sandboxed: runs in its own process (crash-safe, +1 block latency). Click to run in-process."
            : "Runs in-process. Click to sandbox it (crash-safe, +1 block latency)."
        }
        onClick={toggleSandbox}
      >
        ⛉
      </Button>
      {error && (
        <span className="eth-plugin-controls__error" role="alert" title={error}>
          !
        </span>
      )}
    </span>
  );
}
