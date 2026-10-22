import { useEffect, useMemo, useState } from "react";
import { RotateCcw } from "lucide-react";
import { Button, IconButton, NO_AUTOCORRECT, Select, TextInput } from "@/kit";
import {
  COLOR_KEYS,
  contrastFailures,
  CORE_KEYS,
  isColor,
  previewTheme,
  resolveColors,
  THEME_NAME_MAX,
  toHex,
  type ColorKey,
  type CoreKey,
  type ThemeDefinition,
  type ThemeScheme,
} from "@/theme";
import { formatFixed, useT, type MessageKey } from "@/i18n";

const CORE_LABELS: Record<CoreKey, MessageKey> = {
  bg: "settings.appearance.color_bg",
  bgPanel: "settings.appearance.color_bg_panel",
  bgRaised: "settings.appearance.color_bg_raised",
  text: "settings.appearance.color_text",
  textDim: "settings.appearance.color_text_dim",
  accent: "settings.appearance.color_accent",
  ok: "settings.appearance.color_ok",
  warn: "settings.appearance.color_warn",
  danger: "settings.appearance.color_danger",
  info: "settings.appearance.color_info",
};

/** A token's name as the editor and the contrast messages show it: `bgPanel` → `bg-panel`. */
export const tokenLabel = (key: ColorKey): string => key.replace(/[A-Z]/g, (m) => `-${m.toLowerCase()}`);

const isCore = (k: ColorKey): k is CoreKey => (CORE_KEYS as readonly string[]).includes(k);

/** One color: a swatch (the system's color picker) and its value as text (any CSS color). */
function ColorField({
  label,
  name,
  value,
  set,
  onChange,
  onReset,
}: {
  label: string;
  /** The token's name (its test id). */
  name: string;
  value: string;
  /** Set by the theme (not derived): shows the reset button. */
  set: boolean;
  onChange: (value: string) => void;
  onReset?: () => void;
}) {
  const t = useT();
  const [text, setText] = useState(value);
  useEffect(() => setText(value), [value]);
  return (
    <div className="eth-theme-editor__color" data-testid={`theme-color-${name}`} data-set={set || undefined}>
      <span className="eth-theme-editor__label">{label}</span>
      <input type="color" className="eth-theme-editor__swatch" aria-label={label} value={toHex(value) ?? undefined} onChange={(e) => onChange(e.target.value)} />
      <TextInput
        size="sm"
        className="eth-theme-editor__value"
        aria-label={t("settings.appearance.editor_value", { name: label })}
        value={text}
        invalid={!isColor(text)}
        {...NO_AUTOCORRECT}
        onChange={(e) => {
          setText(e.target.value);
          if (isColor(e.target.value)) onChange(e.target.value.trim());
        }}
        onBlur={() => setText(value)}
      />
      {onReset && (
        <IconButton
          size="sm"
          tone="ghost"
          className="eth-theme-editor__reset"
          label={t("settings.appearance.editor_reset", { name: label })}
          icon={<RotateCcw aria-hidden />}
          disabled={!set}
          onClick={onReset}
        />
      )}
    </div>
  );
}

export interface ThemeEditorProps {
  /** The theme being edited (a fresh copy for a new one). */
  draft: ThemeDefinition;
  isNew: boolean;
  onSave: (def: ThemeDefinition) => void;
  onCancel: () => void;
  onExport: (def: ThemeDefinition) => void;
}

/**
 * The theme editor (Settings > Appearance > create your own theme / edit): the name, the base,
 * the main colors, then every other token (derived unless set), and the contrast checks. The
 * whole app previews the draft while it is open (`previewTheme`).
 */
export function ThemeEditor({ draft: initial, isNew, onSave, onCancel, onExport }: ThemeEditorProps) {
  const t = useT();
  const [draft, setDraft] = useState(initial);
  const colors = useMemo(() => resolveColors(draft), [draft]);
  const failures = useMemo(() => contrastFailures(colors), [colors]);
  // Live preview, at most once a frame (a color picker drag sends a stream of changes).
  useEffect(() => {
    const raf = requestAnimationFrame(() => previewTheme(draft));
    return () => cancelAnimationFrame(raf);
  }, [draft]);
  useEffect(() => () => previewTheme(null), []);

  const setColor = (k: ColorKey, v: string | undefined) =>
    setDraft((d) => {
      const next = { ...d.colors };
      if (v === undefined) delete next[k];
      else next[k] = v;
      return { ...d, colors: next };
    });
  const name = draft.name.trim();
  return (
    <div className="eth-theme-editor" data-testid="theme-editor">
      <h4 className="eth-theme-editor__title">{t(isNew ? "settings.appearance.editor_new" : "settings.appearance.editor_edit")}</h4>
      <div className="eth-settings__row">
        <span>{t("settings.appearance.editor_name")}</span>
        <TextInput
          size="sm"
          className="eth-theme-editor__name eth-user-text"
          aria-label={t("settings.appearance.editor_name")}
          value={draft.name}
          maxLength={THEME_NAME_MAX}
          invalid={!name}
          onChange={(e) => setDraft((d) => ({ ...d, name: e.target.value }))}
        />
      </div>
      <div className="eth-settings__row">
        <span>{t("settings.appearance.editor_base")}</span>
        <Select<ThemeScheme>
          size="sm"
          aria-label={t("settings.appearance.editor_base")}
          options={[
            { value: "dark", label: t("settings.appearance.theme_dark") },
            { value: "light", label: t("settings.appearance.theme_light") },
          ]}
          value={draft.base}
          onChange={(base) => setDraft((d) => ({ ...d, base }))}
        />
      </div>
      <p className="eth-settings__hint">{t("settings.appearance.editor_base_hint")}</p>

      <h5 className="eth-theme-editor__subtitle">{t("settings.appearance.editor_main")}</h5>
      <p className="eth-settings__hint">{t("settings.appearance.editor_main_hint")}</p>
      <div className="eth-theme-editor__colors">
        {CORE_KEYS.map((k) => (
          <ColorField key={k} name={tokenLabel(k)} label={t(CORE_LABELS[k])} value={colors[k]} set onChange={(v) => setColor(k, v)} />
        ))}
      </div>

      <section className="eth-theme-editor__contrast" aria-label={t("settings.appearance.editor_contrast")} data-testid="theme-contrast">
        <h5 className="eth-theme-editor__subtitle">{t("settings.appearance.editor_contrast")}</h5>
        {failures.length === 0 ? (
          <p className="eth-settings__hint">{t("settings.appearance.editor_contrast_ok")}</p>
        ) : (
          <ul className="eth-theme-editor__failures">
            {failures.map((f) => (
              <li key={`${f.fg}/${f.bg}`}>
                {t("settings.appearance.editor_contrast_fail", {
                  fg: tokenLabel(f.fg),
                  bg: tokenLabel(f.bg),
                  ratio: formatFixed(f.ratio, 1),
                  min: formatFixed(f.min, 1),
                })}
              </li>
            ))}
          </ul>
        )}
      </section>

      <details className="eth-theme-editor__all">
        <summary className="eth-theme-editor__subtitle">{t("settings.appearance.editor_all")}</summary>
        <p className="eth-settings__hint">{t("settings.appearance.editor_all_hint")}</p>
        <div className="eth-theme-editor__colors">
          {COLOR_KEYS.filter((k) => !isCore(k)).map((k) => (
            <ColorField
              key={k}
              name={tokenLabel(k)}
              label={tokenLabel(k)}
              value={colors[k]}
              set={draft.colors[k] !== undefined}
              onChange={(v) => setColor(k, v)}
              onReset={() => setColor(k, undefined)}
            />
          ))}
        </div>
      </details>

      <div className="eth-theme-editor__actions">
        <Button size="sm" onClick={() => onExport({ ...draft, name: name || initial.name })}>
          {t("settings.appearance.theme_export")}
        </Button>
        <span className="eth-theme-editor__spacer" />
        <Button size="sm" onClick={onCancel}>
          {t("common.cancel")}
        </Button>
        <Button size="sm" tone="accent" disabled={!name} onClick={() => onSave({ ...draft, name })}>
          {t("settings.appearance.editor_save")}
        </Button>
      </div>
    </div>
  );
}
