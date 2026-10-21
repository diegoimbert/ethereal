import { useT } from "@/i18n";

/**
 * A settings section still loading (its code or its data): a heading bar and a few lines, so
 * the page shows at once at about its final height and nothing jumps when the content
 * arrives. `rows`: placeholder lines (a list of rules, of folders...).
 */
export function SettingsPlaceholder({ rows = 2 }: { rows?: number }) {
  const t = useT();
  return (
    <section className="eth-settings__section eth-settings__placeholder" aria-busy="true" aria-label={t("settings.loading")}>
      <span className="eth-settings__placeholder-heading" />
      {Array.from({ length: rows }, (_, i) => (
        <span key={i} className="eth-settings__placeholder-line" />
      ))}
    </section>
  );
}
