import { useState, type CSSProperties, type MouseEvent } from "react";
import { Ellipsis } from "lucide-react";
import { Button, IconButton, openContextMenu, Select, Toggle, useConfirm, type ContextMenuEntry } from "@/kit";
import {
  coreColors,
  deleteCustomTheme,
  findTheme,
  getSchemeTheme,
  getThemeMode,
  isBuiltinTheme,
  newCustomThemeId,
  parseThemeFile,
  resolved,
  saveCustomTheme,
  setSchemeTheme,
  setThemeMode,
  useThemeId,
  useThemeKey,
  useThemes,
  type ThemeDefinition,
  type ThemeFileError,
  type ThemeFileWarning,
  type ThemeScheme,
} from "@/theme";
import { t as tr, useT, type MessageKey } from "@/i18n";
import { ThemeEditor } from "./ThemeEditor";
import { exportThemeFile, openThemeFile } from "./themeFiles";
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

/** A theme's card: a small picture of the app in its colors, its name, and its menu. */
export function ThemeCard({
  def,
  selected,
  onSelect,
  onMenu,
}: {
  def: ThemeDefinition;
  selected: boolean;
  onSelect: () => void;
  onMenu?: (e: MouseEvent) => void;
}) {
  const t = useT();
  const label = useThemeLabel();
  const name = label(def);
  return (
    <div className="eth-theme-card-wrap" onContextMenu={onMenu}>
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
      {onMenu && (
        <IconButton
          size="sm"
          tone="ghost"
          className="eth-theme-card__more"
          label={t("settings.appearance.theme_more", { name })}
          icon={<Ellipsis aria-hidden />}
          onClick={onMenu}
        />
      )}
    </div>
  );
}

const ERRORS: Record<ThemeFileError["code"], MessageKey> = {
  not_json: "settings.appearance.theme_error_not_json",
  not_theme: "settings.appearance.theme_error_not_theme",
  newer_version: "settings.appearance.theme_error_newer",
  bad_version: "settings.appearance.theme_error_bad_version",
  bad_base: "settings.appearance.theme_error_bad_base",
};

function errorText(e: ThemeFileError): string {
  return e.code === "newer_version" ? tr(ERRORS[e.code], { version: e.version }) : tr(ERRORS[e.code]);
}

function warningText(w: ThemeFileWarning, name: string): string {
  if (w.code === "no_name") return tr("settings.appearance.theme_warn_no_name", { name });
  const tokens = w.tokens.join(", ");
  return w.code === "unknown_tokens" ? tr("settings.appearance.theme_warn_unknown", { tokens }) : tr("settings.appearance.theme_warn_invalid", { tokens });
}

type Message = { error: boolean; text: string } | null;
type Editing = { draft: ThemeDefinition; isNew: boolean } | null;

/**
 * Settings > Appearance > theme: the themes as cards (a click applies one; a card's menu edits,
 * duplicates, exports or deletes it), creating and importing themes, the theme editor, and
 * following the system's light or dark (with the theme used for each).
 */
export function ThemePicker() {
  const t = useT();
  const themes = useThemes();
  const [current, setCurrent] = useThemeId();
  // Mode and the per-scheme picks change with the theme: re-read on every theme change.
  useThemeKey();
  const system = getThemeMode() === "system";
  const label = useThemeLabel();
  const [editing, setEditing] = useState<Editing>(null);
  const [message, setMessage] = useState<Message>(null);

  /** A new theme of the user's, starting from `def` (a built-in's main colors, or all of a user's). */
  const copyOf = (def: ThemeDefinition): ThemeDefinition => ({
    id: newCustomThemeId(),
    name: t("settings.appearance.theme_copy_name", { name: label(def) }),
    base: def.base,
    colors: isBuiltinTheme(def.id) ? coreColors(def) : { ...def.colors },
  });
  const startEditing = (e: Editing) => {
    setMessage(null);
    setEditing(e);
  };
  const exportTheme = async (def: ThemeDefinition) => {
    try {
      const path = await exportThemeFile({ ...def, name: label(def) });
      setMessage(path ? { error: false, text: t("settings.appearance.theme_exported", { path }) } : null);
    } catch (e) {
      setMessage({ error: true, text: t("settings.appearance.theme_file_failed", { error: e instanceof Error ? e.message : String(e) }) });
    }
  };
  const importTheme = async () => {
    setMessage(null);
    let text: string | null;
    try {
      text = await openThemeFile();
    } catch (e) {
      setMessage({ error: true, text: t("settings.appearance.theme_file_failed", { error: e instanceof Error ? e.message : String(e) }) });
      return;
    }
    if (text === null) return;
    const parsed = parseThemeFile(text);
    if (!parsed.ok) {
      setMessage({ error: true, text: errorText(parsed.error) });
      return;
    }
    const name = parsed.theme.name || t("settings.appearance.theme_imported_name");
    const def = { ...parsed.theme, id: newCustomThemeId(), name };
    saveCustomTheme(def);
    setCurrent(def.id);
    setMessage({ error: false, text: [t("settings.appearance.theme_imported", { name }), ...parsed.warnings.map((w) => warningText(w, name))].join(" ") });
  };
  const menu = (def: ThemeDefinition) => (e: MouseEvent) => {
    const own = !isBuiltinTheme(def.id);
    const items: ContextMenuEntry[] = [
      ...(own ? [{ label: t("settings.appearance.theme_edit"), onSelect: () => startEditing({ draft: def, isNew: false }) }] : []),
      { label: t("settings.appearance.theme_duplicate"), onSelect: () => startEditing({ draft: copyOf(def), isNew: true }) },
      { label: t("settings.appearance.theme_export"), onSelect: () => void exportTheme(def) },
    ];
    if (own)
      items.push("separator", {
        label: t("settings.appearance.theme_delete"),
        danger: true,
        onSelect: () =>
          useConfirm.getState().ask({
            title: t("settings.appearance.theme_delete_title", { name: def.name }),
            body: t("settings.appearance.theme_delete_body"),
            action: t("settings.appearance.theme_delete"),
            danger: true,
            run: () => deleteCustomTheme(def.id),
          }),
      });
    openContextMenu(e, items);
  };

  if (editing)
    return (
      <ThemeEditor
        key={editing.draft.id}
        draft={editing.draft}
        isNew={editing.isNew}
        onCancel={() => setEditing(null)}
        onExport={(def) => void exportTheme(def)}
        onSave={(def) => {
          saveCustomTheme(def);
          setCurrent(def.id);
          setEditing(null);
        }}
      />
    );

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
  const applied = findTheme(current) ?? themes[0]!;
  return (
    <>
      <div className="eth-theme-grid" role="group" aria-label={t("settings.appearance.theme")}>
        {themes.map((def) => (
          <ThemeCard key={def.id} def={def} selected={def.id === current} onSelect={() => setCurrent(def.id)} onMenu={menu(def)} />
        ))}
      </div>
      <div className="eth-theme-actions">
        <Button size="sm" onClick={() => startEditing({ draft: copyOf(applied), isNew: true })}>
          {t("settings.appearance.theme_create")}
        </Button>
        <Button size="sm" onClick={() => void importTheme()}>
          {t("settings.appearance.theme_import")}
        </Button>
      </div>
      {message && (
        <p className={message.error ? "eth-theme-message eth-theme-message--error" : "eth-theme-message"} role={message.error ? "alert" : "status"}>
          {message.text}
        </p>
      )}
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
