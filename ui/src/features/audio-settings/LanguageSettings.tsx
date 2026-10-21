import { Select } from "@/kit";
import { LOCALES, PSEUDO_LOCALE, setLocale, useLocale, useT, type Locale } from "@/i18n";

/**
 * Settings > General > Language (docs/LOCALIZATION.md): the interface language, kept on this
 * device. It applies at once (no reload). Languages show under their own names; dev builds
 * also offer the pseudo-locale, which marks every translated string.
 */
export function LanguageSettings() {
  const t = useT();
  const locale = useLocale();
  const options: { value: Locale; label: string; userText?: boolean }[] = LOCALES.map((l) => ({ value: l.code, label: l.name }));
  if (import.meta.env.DEV || locale === PSEUDO_LOCALE) options.push({ value: PSEUDO_LOCALE, label: t("settings.language.pseudo") });
  return (
    <section className="eth-settings__section" aria-label={t("settings.language.heading")} data-testid="settings-language">
      {/* The heading names the field: the menu on its line, no second "language" label. */}
      <div className="eth-settings__row">
        <h3 className="eth-settings__heading">{t("settings.language.heading")}</h3>
        <Select<Locale> size="sm" aria-label={t("settings.language.heading")} options={options} value={locale} onChange={(l) => void setLocale(l)} />
      </div>
      <p className="eth-settings__hint">{t("settings.language.hint")}</p>
    </section>
  );
}
