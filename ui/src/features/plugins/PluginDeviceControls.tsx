import { useEffect, useState } from "react";
import type { Command, Device } from "@/generated";
import { RotateCw } from "lucide-react";
import { Badge, Button, IconButton, openContextMenu, type ContextMenuEntry } from "@/kit";
import { resolvePluginEntries } from "@/features/import-resolver/menus";
import { useProjectStore } from "@/state";
import { cmd, useTransport, type EngineTransport } from "@/transport";
import { t as tr, useT } from "@/i18n";
import { FORMAT_LABEL, isInstalled } from "./filter";
import { useEnginePlugins, usePluginEvents, usePluginStore } from "./pluginStore";
import { isSandboxDefault, useSandboxDefaults } from "./sandboxDefaults";

export interface PluginDeviceControlsProps {
  device: Device;
}

/**
 * Plugin-specific controls in a device header (mounted by `DeviceView` for every device;
 * renders nothing for built-ins): the safe-mode placeholder, crashed and missing states (bypassed) with their Load / Reload
 * action, and "found · reload" once a rescan lists a plugin that was missing (it stays bypassed until reloaded, with
 * the state saved in the project). Works the same for every format. Sandboxing is in the device's context menu
 * (`pluginMenuEntries`). The editor window opens from the header's window button, the same
 * for every device (`features/devices`; `setPluginWindow` below).
 */
export function PluginDeviceControls({ device }: PluginDeviceControlsProps) {
  const t = useT();
  usePluginEvents();
  const transport = useTransport();
  const crash = usePluginStore((s) => s.crashed[device.id]);
  const clearCrash = usePluginStore((s) => s.clearCrash);
  const found = usePluginStore((s) => !!s.found[device.id]);
  const clearFound = usePluginStore((s) => s.clearFound);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Where the engine runs: another computer's list while its engine plays the project.
  const plugins = useEnginePlugins(device.kind.type === "Plugin");
  // Held as a placeholder (the project was opened without plugins).
  const held = useProjectStore((s) => s.safeMode.includes(device.id));
  const isPlugin = device.kind.type === "Plugin";
  // The device menu's "always sandbox" reads the engine host's list: load it once.
  useEffect(() => {
    if (isPlugin) void useSandboxDefaults.getState().load(transport);
  }, [isPlugin, transport]);

  if (device.kind.type !== "Plugin") return null;
  const { format, plugin_id } = device.kind.plugin;
  // Not in the engine host's list: it couldn't load it and the device is bypassed.
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

  // Re-instantiates the plugin (from its last saved state).
  const reload = () =>
    run(cmd("Plugin", { type: "Reload", device: device.id }), () => {
      clearCrash(device.id);
      clearFound(device.id);
    });
  const load = () =>
    run(cmd("Plugin", { type: "Reload", device: device.id }), () => {
      const store = useProjectStore.getState();
      store.setSafeMode(
        store.safe,
        store.safeMode.filter((d) => d !== device.id),
      );
    });

  return (
    <span className="eth-plugin-controls" data-plugin-controls={device.id}>
      {held ? (
        <>
          <Badge tone="warn" className="eth-plugin-controls__missing">
            <span role="status" title={t("plugins.device.safe_mode_hint")}>
              {t("plugins.device.safe_mode")}
            </span>
          </Badge>
          <IconButton
            size="sm"
            tone="ghost"
            disabled={busy}
            label={t("plugins.device.load", { name })}
            title={t("plugins.device.load_hint")}
            icon={<RotateCw />}
            onClick={load}
          />
        </>
      ) : missing ? (
        <>
          <Badge tone="warn" className="eth-plugin-controls__missing">
            <span
              role="status"
              title={t("plugins.device.missing_hint", { format: FORMAT_LABEL[format], id: plugin_id })}
              // Right-click "Resolve…" (a replacement plugin).
              onContextMenu={(e) => openContextMenu(e, resolvePluginEntries(device, transport))}
            >
              {t("plugins.device.missing")}
            </span>
          </Badge>
          <IconButton
            size="sm"
            tone="ghost"
            disabled={busy}
            label={t("plugins.device.reload", { name })}
            title={t("plugins.device.reload_missing_hint")}
            icon={<RotateCw />}
            onClick={reload}
          />
        </>
      ) : found ? (
        <Button
          size="sm"
          variant="ghost"
          className="eth-plugin-controls__found"
          disabled={busy}
          aria-label={t("plugins.device.reload", { name })}
          title={t("plugins.device.found_hint", { format: FORMAT_LABEL[format], id: plugin_id })}
          onClick={reload}
        >
          {t("plugins.device.found")}
        </Button>
      ) : crash !== undefined ? (
        <>
          <span className="eth-plugin-controls__crashed" role="status" title={crash}>
            {t("plugins.device.crashed")}
          </span>
          <IconButton
            size="sm"
            tone="ghost"
            disabled={busy}
            label={t("plugins.device.reload", { name })}
            title={t("plugins.device.reload_crashed_hint")}
            icon={<RotateCw />}
            onClick={reload}
          />
        </>
      ) : null}
      {error && (
        <span className="eth-plugin-controls__error" role="alert" title={error}>
          !
        </span>
      )}
    </span>
  );
}

/**
 * The plugin's entries of its device context menu (advanced): sandboxing, which runs the
 * plugin in its own process (crash-safe, +1 block latency) and re-instantiates it. When the
 * engine host has the "sandboxed by default" setting (`./sandboxDefaults.ts`, loaded by the
 * device header), "always sandbox this plugin" adds it to that list and sandboxes this
 * instance too; other instances keep their own setting. Once listed, the entry takes it off
 * the list (this instance stays as it is).
 */
export function pluginMenuEntries(device: Device, transport: EngineTransport): ContextMenuEntry[] {
  if (device.kind.type !== "Plugin") return [];
  const { sandboxed, format, plugin_id, name } = device.kind.plugin;
  const setSandboxed = (on: boolean) =>
    transport.send(cmd("Plugin", { type: "SetSandboxed", device: device.id, sandboxed: on })).then(() => usePluginStore.getState().clearCrash(device.id));
  const entries: ContextMenuEntry[] = [
    {
      label: sandboxed ? tr("plugins.device.run_in_process") : tr("plugins.device.run_sandboxed"),
      onSelect: () => void setSandboxed(!sandboxed).catch((e: unknown) => console.warn("sandbox toggle failed", e)),
    },
  ];
  const defaults = useSandboxDefaults.getState();
  if (defaults.from === transport && defaults.list) {
    const listed = isSandboxDefault(defaults.list, { format, plugin_id });
    entries.push({
      label: listed ? tr("plugins.device.stop_always_sandbox") : tr("plugins.device.always_sandbox"),
      onSelect: () =>
        void defaults
          .set(transport, { format, plugin_id, name: name || device.name }, !listed)
          .then(() => (!listed && !sandboxed ? setSandboxed(true) : undefined))
          .catch((e: unknown) => console.warn("sandbox default failed", e)),
    });
  }
  return entries;
}

/** Open or close a plugin's editor window (the store follows on success). */
export function setPluginWindow(transport: EngineTransport, device: Device, open: boolean): void {
  if (device.kind.type !== "Plugin") return;
  const store = usePluginStore.getState();
  if (store.crashed[device.id] !== undefined || !!store.editors[device.id] === open) return;
  transport
    .send(cmd("Plugin", { type: open ? "OpenEditor" : "CloseEditor", device: device.id }))
    .then(() => usePluginStore.getState().setEditorOpen(device.id, open))
    .catch((e: unknown) => console.warn("plugin window failed", e));
}
