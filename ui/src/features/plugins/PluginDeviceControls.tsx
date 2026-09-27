import { useState } from "react";
import type { Command, Device } from "@/generated";
import { Button } from "@/kit";
import { cmd, useTransport } from "@/transport";
import { usePluginEvents, usePluginStore } from "./pluginStore";

export interface PluginDeviceControlsProps {
  device: Device;
}

/**
 * Plugin-specific controls in a device header (mounted by `DeviceView` for every device;
 * renders nothing for built-ins): editor window, sandbox toggle, and the crashed state
 * with its Reload action.
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

  if (device.kind.type !== "Plugin") return null;
  const { sandboxed } = device.kind.plugin;
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

  return (
    <span className="eth-plugin-controls" data-plugin-controls={device.id}>
      {crash !== undefined ? (
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
