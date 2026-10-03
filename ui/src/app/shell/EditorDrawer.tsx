import { useEffect, type KeyboardEvent } from "react";
import { matchesAction } from "@/features/keymap";
import { Button } from "@/kit";
import { useShellStore } from "./shellStore";
import { DRAWER_TABS } from "./tabs";

/** The drawer's tabs (shown in the pane header). */
export function DrawerTabs() {
  const tab = useShellStore((s) => s.bottom.tab);
  const open = useShellStore((s) => s.openDrawer);
  return (
    <div className="eth-shell__tabs" role="tablist" aria-label="Editors">
      {DRAWER_TABS.map((t) => (
        <Button
          key={t.id}
          size="sm"
          tone="ghost"
          role="tab"
          className="eth-shell__tab"
          aria-selected={t.id === tab}
          onClick={() => open(t.id)}
        >
          {t.label}
        </Button>
      ))}
    </div>
  );
}

/**
 * The active editor. Escape closes the drawer (unless the editor used the key itself, e.g.
 * the piano roll deselecting notes).
 */
export function EditorDrawer() {
  const tab = useShellStore((s) => s.bottom.tab);
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key !== "Escape" || e.defaultPrevented) return;
    e.preventDefault();
    useShellStore.getState().setOpen("bottom", false);
  };
  return (
    <div className="eth-drawer" data-slot="detail" onKeyDown={onKeyDown}>
      {(DRAWER_TABS.find((t) => t.id === tab) ?? DRAWER_TABS[0]!).render()}
    </div>
  );
}

/**
 * ⌘J / Ctrl+J (keymap `view.editor`) toggles the editor drawer (anywhere but text fields). Escape that nothing else
 * used (not in a text field, dialog or menu) closes an unpinned piano roll. Renders nothing.
 */
export function DrawerShortcut() {
  useEffect(() => {
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (matchesAction("edit.deselect", e) && !e.defaultPrevented) {
        const t = e.target as HTMLElement | null;
        if (t?.closest?.('input, textarea, [contenteditable="true"], [role="dialog"], [role="menu"], [role="listbox"]')) return;
        const { bottom, setOpen } = useShellStore.getState();
        if (bottom.open && !bottom.pinned && bottom.tab === "piano-roll") setOpen("bottom", false);
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
