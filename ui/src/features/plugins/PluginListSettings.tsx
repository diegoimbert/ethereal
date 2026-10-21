/**
 * Settings > Plugins > Plugin list: which scanned plugins the device library lists
 * (`./visibility.ts`, kept on this device): one format per plugin (on by default, the
 * preferred format first), and the hidden plugins (every format or one), each with "show
 * again"; "hide a plugin…" adds one (a library row's right-click does too).
 */
import { EyeOff, X } from "lucide-react";
import type { PluginFormat } from "@/generated";
import { DevicePicker } from "@/features/devices/DevicePicker";
import { Button, IconButton, Select, Toggle, useConfirm } from "@/kit";
import { useT } from "@/i18n";
import { FORMAT_LABEL } from "./filter";
import { usePluginStore } from "./pluginStore";
import { DEDUPE_ORDER, usePluginVisibility, visiblePlugins } from "./visibility";

const ALL_FORMATS = "all";

export function PluginListSettings() {
  const t = useT();
  const v = usePluginVisibility();
  const plugins = usePluginStore((s) => s.plugins);
  const total = plugins?.length ?? 0;
  const listed = plugins ? visiblePlugins(plugins, v).length : 0;
  const formatOptions = DEDUPE_ORDER.map((f) => ({ value: f, label: FORMAT_LABEL[f] }));
  const reset = () =>
    useConfirm.getState().ask({
      title: t("plugins.visibility.reset_title"),
      body: t("plugins.visibility.reset_body"),
      action: t("plugins.visibility.reset"),
      danger: true,
      run: () => usePluginVisibility.getState().reset(),
    });
  return (
    <div className="eth-settings__stack" data-testid="settings-plugin-list">
      <section className="eth-settings__section" aria-label={t("plugins.visibility.formats")}>
        <h3 className="eth-settings__heading">{t("plugins.visibility.formats")}</h3>
        <Toggle size="sm" checked={v.dedupe} onChange={v.setDedupe} label={t("plugins.visibility.dedupe")} />
        <p className="eth-settings__hint">{t("plugins.visibility.dedupe_hint")}</p>
        <div className="eth-settings__row">
          <span>{t("plugins.visibility.preferred")}</span>
          <Select
            size="sm"
            aria-label={t("plugins.visibility.preferred")}
            disabled={!v.dedupe}
            options={formatOptions}
            value={v.preferred}
            onChange={(f) => v.setPreferred(f as PluginFormat)}
          />
        </div>
        {total > 0 && <p className="eth-settings__hint">{t("plugins.visibility.count", { listed, total })}</p>}
      </section>
      <section className="eth-settings__section" aria-label={t("plugins.visibility.hidden")}>
        <div className="eth-settings__row">
          <h3 className="eth-settings__heading">{t("plugins.visibility.hidden")}</h3>
          <Button size="sm" disabled={v.rules.length === 0 && v.dedupe && v.preferred === "Clap"} onClick={reset}>
            {t("plugins.visibility.reset")}
          </Button>
        </div>
        <p className="eth-settings__hint">{t("plugins.visibility.hidden_hint")}</p>
        {v.rules.length > 0 && (
          <ul className="eth-plugin-list__rules" aria-label={t("plugins.visibility.hidden")}>
            {v.rules.map((r, i) => (
              <li key={`${r.vendor}|${r.name}|${r.format ?? ""}`} className="eth-plugin-list__rule">
                <EyeOff aria-hidden className="eth-plugin-list__icon" />
                <span className="eth-plugin-list__name eth-user-text">{r.name}</span>
                {r.vendor && <span className="eth-plugin-list__vendor eth-user-text">{r.vendor}</span>}
                <Select
                  size="sm"
                  aria-label={t("plugins.visibility.rule_format", { name: r.name })}
                  options={[{ value: ALL_FORMATS, label: t("plugins.visibility.every_format") }, ...formatOptions]}
                  value={r.format ?? ALL_FORMATS}
                  onChange={(f) => v.setRule(i, { ...r, format: f === ALL_FORMATS ? null : (f as PluginFormat) })}
                />
                <IconButton size="sm" tone="ghost" label={t("plugins.visibility.show_again", { name: r.name })} icon={<X />} onClick={() => v.removeRule(i)} />
              </li>
            ))}
          </ul>
        )}
        <DevicePicker
          aria-label={t("plugins.visibility.hide_one")}
          className="eth-plugin-list__add"
          placeholder
          pluginsOnly
          onPick={(p) => {
            const plugin = p.device.type === "Plugin" ? p.device : null;
            const d = plugin && plugins?.find((x) => x.format === plugin.format && x.id === plugin.plugin_id);
            if (d) v.hide({ name: d.name, vendor: d.vendor, format: null });
          }}
        >
          {t("plugins.visibility.hide_one")}
        </DevicePicker>
      </section>
    </div>
  );
}
