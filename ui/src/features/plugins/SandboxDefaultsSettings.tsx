/**
 * Settings > Plugins > Sandboxing: the plugins inserted sandboxed by default
 * (`./sandboxDefaults.ts`), kept by the engine host (this computer, or the one whose engine
 * plays the project). "add a plugin…" picks one from the scanned list (searchable), each row
 * removes one. It only picks how a new instance starts: each device keeps its own setting
 * (its menu's "run sandboxed / in-process"), saved in the project, and editing this list
 * never changes a device already there.
 */
import { useEffect, useState } from "react";
import { Shield, X } from "lucide-react";
import { DevicePicker } from "@/features/devices/DevicePicker";
import { SettingsPlaceholder } from "@/features/audio-settings/SettingsPlaceholder";
import { Badge, IconButton } from "@/kit";
import { useT } from "@/i18n";
import { commandErrorText, isCommandFailed, type EngineTransport } from "@/transport";
import { FORMAT_LABEL } from "./filter";
import { isSandboxDefault, useSandboxDefaults } from "./sandboxDefaults";

function errorText(e: unknown): string {
  if (isCommandFailed(e)) return commandErrorText(e.error);
  return e instanceof Error ? e.message : String(e);
}

export function SandboxDefaultsSettings({ transport }: { transport: EngineTransport }) {
  const t = useT();
  const list = useSandboxDefaults((s) => (s.from === transport ? s.list : null));
  const unsupported = useSandboxDefaults((s) => s.from === transport && s.unsupported);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    void useSandboxDefaults.getState().load(transport, true);
  }, [transport]);

  if (unsupported) return <p className="eth-settings__hint">{t("plugins.sandbox_defaults.unsupported")}</p>;
  if (!list) return <SettingsPlaceholder rows={3} />;
  const edit = (plugin: { format: (typeof list)[number]["format"]; plugin_id: string; name?: string }, on: boolean) => {
    setError(null);
    useSandboxDefaults
      .getState()
      .set(transport, plugin, on)
      .catch((e: unknown) => setError(errorText(e)));
  };
  return (
    <div className="eth-settings__stack" data-testid="settings-sandbox-defaults">
      <section className="eth-settings__section" aria-label={t("plugins.sandbox_defaults.title")}>
        <h3 className="eth-settings__heading">{t("plugins.sandbox_defaults.title")}</h3>
        <p className="eth-settings__hint">{t("plugins.sandbox_defaults.hint")}</p>
        {error && (
          <p className="eth-settings__hint" role="alert">
            {error}
          </p>
        )}
        {list.length === 0 ? (
          <p className="eth-settings__hint">{t("plugins.sandbox_defaults.empty")}</p>
        ) : (
          <ul className="eth-plugin-list__rules" aria-label={t("plugins.sandbox_defaults.title")}>
            {list.map((d) => (
              <li key={`${d.format}|${d.plugin_id}`} className="eth-plugin-list__rule">
                <Shield aria-hidden className="eth-plugin-list__icon" />
                <span className="eth-plugin-list__name eth-user-text">{d.name}</span>
                <span className="eth-plugin-list__vendor">
                  <Badge>{FORMAT_LABEL[d.format]}</Badge>
                </span>
                <IconButton size="sm" tone="ghost" label={t("plugins.sandbox_defaults.remove", { name: d.name })} icon={<X />} onClick={() => edit(d, false)} />
              </li>
            ))}
          </ul>
        )}
        <DevicePicker
          aria-label={t("plugins.sandbox_defaults.add")}
          className="eth-plugin-list__add"
          placeholder
          pluginsOnly
          onPick={(p) => {
            if (p.device.type !== "Plugin" || !p.device.format) return;
            const plugin = { format: p.device.format, plugin_id: p.device.plugin_id, name: p.name };
            if (!isSandboxDefault(list, plugin)) edit(plugin, true);
          }}
        >
          {t("plugins.sandbox_defaults.add")}
        </DevicePicker>
      </section>
    </div>
  );
}
