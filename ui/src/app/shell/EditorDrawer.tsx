import { Maximize2, Minimize2 } from "lucide-react";
import { useEffect, type KeyboardEvent } from "react";
import { matchesAction, useShortcutLabel } from "@/features/keymap";
import { useT } from "@/i18n";
import { Button, IconButton } from "@/kit";
import { drawerCanFullscreen, useShellStore } from "@/state/shell";
import { DRAWER_TABS } from "./tabs";
import { useClipEditorLabel } from "@/features/piano-roll";
import { useClip, useEditedClipId } from "@/state";
import type { DrawerTab } from "@/state/shell";

/** "Piano Roll", or "Beat editor" while the open clip shows the step sequencer. */
function ClipEditorLabel() {
  return <>{useClipEditorLabel()}</>;
}

/** The tabs that apply to the edited clip: the clip editor for a MIDI clip, Warp for an audio one. */
function useDrawerTabs() {
  const kind = useClip(useEditedClipId())?.content.type;
  return DRAWER_TABS.filter((t) => (t.id === "piano-roll" ? kind === "Midi" : t.id === "warp" ? kind === "Audio" : true));
}

/** The shown tab: the chosen one, or the first when it doesn't apply (e.g. Warp on a MIDI clip). */
function useActiveTab(tabs: ReturnType<typeof useDrawerTabs>): DrawerTab {
  const tab = useShellStore((s) => s.bottom.tab);
  return tabs.some((t) => t.id === tab) ? tab : tabs[0]!.id;
}

/**
 * Whether the drawer is shown fullscreen: asked for, open and expanded, and showing the clip
 * editor (piano roll / beat editor).
 */
export function useDrawerFullscreen(): boolean {
  const tab = useActiveTab(useDrawerTabs());
  const on = useShellStore((s) => s.drawerFullscreen && s.bottom.open && !s.bottom.collapsed);
  return on && drawerCanFullscreen(tab);
}

/** Header button: the clip editor (piano roll / beat editor) fills the workspace, or back. */
export function DrawerFullscreenButton() {
  const t = useT();
  const tab = useActiveTab(useDrawerTabs());
  const on = useDrawerFullscreen();
  const key = useShortcutLabel("view.editorFullscreen");
  if (!drawerCanFullscreen(tab)) return null;
  return (
    <IconButton
      size="sm"
      tone="ghost"
      active={on}
      label={on ? t("shell.pane.exit_fullscreen") : t("shell.pane.fullscreen")}
      title={on ? t("shell.pane.exit_fullscreen_title") : key ? t("shell.pane.fullscreen_key", { key }) : t("shell.pane.fullscreen")}
      icon={on ? <Minimize2 /> : <Maximize2 />}
      onClick={() => useShellStore.getState().setDrawerFullscreen(!on)}
    />
  );
}

/** The drawer's tabs (shown in the pane header). */
export function DrawerTabs() {
  const t = useT();
  const tabs = useDrawerTabs();
  const tab = useActiveTab(tabs);
  const open = useShellStore((s) => s.openDrawer);
  // The shown tab fell back from the clip editor (an audio clip opened): leave fullscreen.
  const stale = useShellStore((s) => s.drawerFullscreen) && !drawerCanFullscreen(tab);
  useEffect(() => {
    if (stale) useShellStore.getState().setDrawerFullscreen(false);
  }, [stale]);
  return (
    <div className="eth-shell__tabs" role="tablist" aria-label={t("shell.drawer.tabs")}>
      {tabs.map((d) => (
        <Button key={d.id} size="sm" tone="ghost" role="tab" className="eth-shell__tab" aria-selected={d.id === tab} onClick={() => open(d.id)}>
          {d.id === "piano-roll" ? <ClipEditorLabel /> : t(d.label)}
        </Button>
      ))}
    </div>
  );
}

/**
 * The active editor. Escape leaves fullscreen, else closes the drawer (unless the editor used
 * the key itself, e.g. the piano roll deselecting notes).
 */
export function EditorDrawer() {
  const tab = useActiveTab(useDrawerTabs());
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key !== "Escape" || e.defaultPrevented) return;
    e.preventDefault();
    const shell = useShellStore.getState();
    if (shell.drawerFullscreen) shell.setDrawerFullscreen(false);
    else shell.setOpen("bottom", false);
  };
  return (
    <div className="eth-drawer" data-slot="detail" data-tour="editor" onKeyDown={onKeyDown}>
      {(DRAWER_TABS.find((t) => t.id === tab) ?? DRAWER_TABS[0]!).render()}
    </div>
  );
}

/**
 * ⌘⌥L / Ctrl+Alt+L (keymap `view.editor`; ⌘J consolidates clips since #870) toggles the editor drawer (anywhere but text fields). Escape that nothing else
 * used (not in a text field, dialog or menu) leaves the drawer's fullscreen, else closes an unpinned drawer.
 * Renders nothing.
 */
export function DrawerShortcut() {
  useEffect(() => {
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (matchesAction("edit.deselect", e) && !e.defaultPrevented) {
        const t = e.target as HTMLElement | null;
        if (t?.closest?.('input, textarea, [contenteditable="true"], [role="dialog"], [role="menu"], [role="listbox"]')) return;
        const { bottom, setOpen, drawerFullscreen, setDrawerFullscreen } = useShellStore.getState();
        if (drawerFullscreen) {
          e.preventDefault();
          setDrawerFullscreen(false);
        } else if (bottom.open && !bottom.pinned) setOpen("bottom", false);
        return;
      }
      if (!matchesAction("view.editor", e)) return;
      const t = e.target as HTMLElement | null;
      if (t && (t.isContentEditable || t.tagName === "INPUT" || t.tagName === "TEXTAREA")) return;
      e.preventDefault();
      const shell = useShellStore.getState();
      shell.setOpen("bottom", !shell.bottom.open);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  return null;
}
