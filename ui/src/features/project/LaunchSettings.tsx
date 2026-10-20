import { Toggle } from "@/kit";
import { useT } from "@/i18n";
import { useLaunchPrefs } from "./launch";

/** Settings > General: what happens on launch. Appearance has its own tab. */
export function LaunchSettings() {
  const t = useT();
  const reopen = useLaunchPrefs((s) => s.reopenLast);
  const setReopen = useLaunchPrefs((s) => s.setReopenLast);
  return (
    <div className="eth-settings__stack" data-testid="settings-general">
      <section className="eth-settings__section" aria-label={t("settings.startup.heading")}>
        <h3 className="eth-settings__heading">{t("settings.startup.heading")}</h3>
        <Toggle size="sm" checked={reopen} onChange={setReopen} label={t("settings.startup.reopen")} />
        <p className="eth-settings__hint">{t("settings.startup.reopen_hint")}</p>
      </section>
    </div>
  );
}
