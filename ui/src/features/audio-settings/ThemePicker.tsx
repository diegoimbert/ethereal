import type { CSSProperties } from "react";
import { Select, Toggle } from "@/kit";
import {
  getSchemeTheme,
  getThemeMode,
  isBuiltinTheme,
  resolved,
  setSchemeTheme,
  setThemeMode,
  useThemeId,
  useThemeKey,
  useThemes,
  type ThemeDefinition,
  type ThemeScheme,
} from "@/theme";
import { useT, type MessageKey } from "@/i18n";
import { useThemeLabel } from "./themeLabel";

/** The card preview's colors, as custom properties (the theme isn't applied to the card). */
function previewStyle(def: ThemeDefinition): CSSProperties {
  const c = resolved(def).color;
  return {
    "--preview-bg": c.bg,
    "--preview-panel": c.bgPanel,
    "--preview-raised": c.bgRaised,
    "--preview-border": c.borderLight,
    "--preview-text": c.text,
    "--preview-dim": c.textDim,
    "--preview-accent": c.accent,
    "--preview-ok": c.ok,
  } as CSSProperties;
}

/** A theme's card: a small picture of the app in its colors, and its name. */
export function ThemeCard({ def, selected, onSelect }: { def: ThemeDefinition; selected: boolean; onSelect: () => void }) {
  const label = useThemeLabel();
  const name = label(def);
  return (
    <button type="button" className="eth-theme-card" aria-pressed={selected} data-testid={`theme-card-${def.id}`} onClick={onSelect}>
      <span className="eth-theme-card__preview" style={previewStyle(def)} aria-hidden>
        <span className="eth-theme-card__bar">
          <span className="eth-theme-card__dot" />
        </span>
        <span className="eth-theme-card__pane">
          <span className="eth-theme-card__line" />
          <span className="eth-theme-card__line eth-theme-card__line--dim" />
          <span className="eth-theme-card__button" />
        </span>
        <span className="eth-theme-card__clip" />
      </span>
      <span className={isBuiltinTheme(def.id) ? "eth-theme-card__name" : "eth-theme-card__name eth-user-text"}>{name}</span>
    </button>
  );
}

/**
 * Settings > Appearance > theme: the themes as cards, and following the system's light or dark
 * (with the theme used for each).
 */
export function ThemePicker() {
  const t = useT();
  const themes = useThemes();
  const [current, setCurrent] = useThemeId();
  // Mode and the per-scheme picks change with the theme: re-read on every theme change.
  useThemeKey();
  const system = getThemeMode() === "system";
  const label = useThemeLabel();
  const schemeSelect = (scheme: ThemeScheme, key: MessageKey) => (
    <div className="eth-settings__row">
      <span>{t(key)}</span>
      <Select
        size="sm"
        aria-label={t(key)}
        options={themes.filter((d) => d.base === scheme).map((d) => ({ value: d.id, label: label(d) }))}
        value={getSchemeTheme(scheme)}
        onChange={(id) => setSchemeTheme(scheme, id)}
      />
    </div>
  );
  return (
    <>
      <div className="eth-theme-grid" role="group" aria-label={t("settings.appearance.theme")}>
        {themes.map((def) => (
          <ThemeCard key={def.id} def={def} selected={def.id === current} onSelect={() => setCurrent(def.id)} />
        ))}
      </div>
      <Toggle size="sm" checked={system} onChange={(on) => setThemeMode(on ? "system" : "fixed")} label={t("settings.appearance.theme_system")} />
      {system && (
        <>
          {schemeSelect("light", "settings.appearance.theme_system_light")}
          {schemeSelect("dark", "settings.appearance.theme_system_dark")}
        </>
      )}
      <p className="eth-settings__hint">{t("settings.appearance.theme_system_hint")}</p>
    </>
  );
}
