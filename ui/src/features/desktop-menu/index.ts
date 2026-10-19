// The desktop app's macOS menu bar (#573; the shell's side: apps/desktop/src-tauri/src/app_menu.rs).
// Its File, Edit (undo / redo / select all) and Help items emit `MENU_ACTION_EVENT` with a
// keymap action id or a command palette id; `runMenuAction` runs it as its shortcut or palette
// row would. Undo / redo / select all in a text field go back to AppKit (the field's own
// editing). The keyed items' equivalents follow the keymap (`menuShortcuts`). The web build
// has no bridge and mounts nothing.
import { contributions } from "@/ext";
import { useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { isTextEntry } from "@/features/transport-bar/engine";
import { chordsFor, dispatchPaused, paletteEntries, runAction, useKeymapStore } from "@/features/keymap";
import { findAction } from "@/features/keymap/store";
import { isTauri } from "@/transport";
import { subscribeLocale, t, type MessageKey } from "@/i18n";

export const MENU_ACTION_EVENT = "ether://menu-action";

/** The items whose key equivalent the keymap sets (the shell's `keyed_actions`). */
export const MENU_KEYED_ACTIONS = ["edit.undo", "edit.redo", "edit.selectAll"] as const;

/** Edit actions that, in a text field, are the field's own (AppKit's `undo:` …). */
const TEXT_FIELD_ACTIONS = new Set<string>(["edit.undo", "edit.redo", "edit.selectAll"]);

export interface MenuBridge {
  /** Calls `handler` with each chosen item's action id; resolves with the unsubscribe. */
  onAction(handler: (id: string) => void): Promise<() => void>;
  /** The keyed items' chords (keymap syntax; null: unbound). */
  setShortcuts(shortcuts: ReadonlyArray<{ action: string; chord: string | null }>): Promise<void>;
  /** Undo / redo / select all of the focused text field (AppKit). */
  nativeEdit(action: string): Promise<void>;
  /** The menu bar's text in the UI's language (`menu_set_labels`). */
  setLabels?(labels: MenuLabels): Promise<void>;
}

/**
 * The app items' labels, by action id (the shell's `FILE_ITEMS`, `EDIT_*`, `HELP_ITEMS`). Items
 * an extension adds to the shell's menus are labelled by it (`@/ext` "desktopMenu.labels").
 */
const ACTION_LABELS: Record<string, MessageKey> = {
  "project:new": "menu.action.project_new",
  "project:open": "menu.action.project_open",
  "project.save": "menu.action.project_save",
  "project:save-as": "menu.action.project_save_as",
  "file.import": "menu.action.file_import",
  "project:export-audio": "menu.action.project_export_audio",
  "audio-settings": "menu.action.audio_settings",
  "edit.undo": "menu.action.edit_undo",
  "edit.redo": "menu.action.edit_redo",
  "edit.selectAll": "menu.action.edit_select_all",
  "keymap:open": "menu.action.keymap_open",
  "app:news": "menu.action.app_news",
};

/** The menu bar's text (`MenuLabels` in app_menu.rs). */
export type MenuLabels = ReturnType<typeof menuLabels>;

/** The menu bar's text in the current language. */
export function menuLabels() {
  const app = "ethereal";
  return {
    about: t("menu.app.about", { app }),
    services: t("menu.app.services"),
    hide: t("menu.app.hide", { app }),
    hide_others: t("menu.app.hide_others"),
    quit: t("menu.app.quit", { app }),
    file: t("menu.file"),
    close_window: t("menu.file.close_window"),
    edit: t("menu.edit"),
    cut: t("menu.edit.cut"),
    copy: t("menu.edit.copy"),
    paste: t("menu.edit.paste"),
    view: t("menu.view"),
    fullscreen: t("menu.view.fullscreen"),
    window: t("menu.window"),
    minimize: t("menu.window.minimize"),
    zoom: t("menu.window.zoom"),
    help: t("menu.help"),
    actions: {
      ...Object.fromEntries(Object.entries(ACTION_LABELS).map(([action, key]) => [action, t(key)])),
      ...Object.assign({}, ...contributions("desktopMenu.labels").map((labels) => labels())),
    } as Record<string, string>,
  };
}

export const tauriMenuBridge: MenuBridge = {
  onAction: (handler) => listen<string>(MENU_ACTION_EVENT, (e) => handler(e.payload)),
  setShortcuts: (shortcuts) => invoke<void>("menu_set_shortcuts", { shortcuts }),
  nativeEdit: (action) => invoke<void>("menu_native_edit", { action }),
  setLabels: (labels) => invoke<void>("menu_set_labels", { labels }),
};

let override: MenuBridge | null | undefined;

/** Tests: use `bridge` (null: as in the web build); `undefined` restores the default. */
export function setMenuBridge(bridge: MenuBridge | null | undefined): void {
  override = bridge;
}

export function menuBridge(): MenuBridge | null {
  if (override !== undefined) return override;
  return isTauri() ? tauriMenuBridge : null;
}

/** Run the menu item `id`: a keymap action where its chord would run, else a palette command. */
export function runMenuAction(id: string, bridge: MenuBridge | null = menuBridge()): void {
  // The keymap editor is recording a chord: the key the menu took was meant for it.
  if (dispatchPaused()) return;
  if (TEXT_FIELD_ACTIONS.has(id) && isTextEntry(document.activeElement)) {
    void bridge?.nativeEdit(id).catch(() => undefined);
    return;
  }
  if (findAction(id)) {
    runAction(id);
    return;
  }
  paletteEntries()
    .find((e) => e.id === id)
    ?.run();
}

/** The keyed items' current chords (the first of each action's, as the keymap stores it). */
export function menuShortcuts(): Array<{ action: string; chord: string | null }> {
  return MENU_KEYED_ACTIONS.map((action) => ({ action, chord: chordsFor(action)[0] ?? null }));
}

/** Mounted once by the app shell: runs the menu's items and keeps their key equivalents in sync. */
export function DesktopMenuRoot(): null {
  useEffect(() => {
    const bridge = menuBridge();
    if (!bridge) return;
    let off: (() => void) | null = null;
    let alive = true;
    void bridge
      .onAction((id) => runMenuAction(id, bridge))
      .then((unlisten) => (alive ? (off = unlisten) : unlisten()))
      .catch(() => undefined);
    let last = "";
    const sync = () => {
      const shortcuts = menuShortcuts();
      const key = JSON.stringify(shortcuts);
      if (key === last) return;
      last = key;
      void bridge.setShortcuts(shortcuts).catch(() => undefined);
    };
    sync();
    const unsub = useKeymapStore.subscribe(sync);
    // The menu bar follows the UI's language (now, and on every switch).
    const relabel = () => void bridge.setLabels?.(menuLabels()).catch(() => undefined);
    relabel();
    const unsubLocale = subscribeLocale(relabel);
    return () => {
      alive = false;
      off?.();
      unsub();
      unsubLocale();
    };
  }, []);
  return null;
}
