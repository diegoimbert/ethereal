// #573: the macOS menu bar's items run the UI's own actions; their key equivalents follow the keymap.
import { act, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { matchesAction, pauseDispatch, setPaletteSource } from "@/features/keymap";
import { DEFAULT_KEYMAP, useKeymapStore, withBinding } from "@/features/keymap/store";
import { resetI18nForTests, setLocale } from "@/i18n/runtime";
import { DesktopMenuRoot, setMenuBridge, type MenuBridge, type MenuLabels } from ".";

function fakeBridge() {
  let handler: ((id: string) => void) | null = null;
  const bridge = {
    onAction: vi.fn(async (h: (id: string) => void) => {
      handler = h;
      return () => {
        handler = null;
      };
    }),
    setShortcuts: vi.fn(async () => {}),
    nativeEdit: vi.fn(async () => {}),
    setLabels: vi.fn(async (_labels: MenuLabels) => {}),
  } satisfies MenuBridge;
  return { bridge, choose: (id: string) => act(() => handler?.(id)) };
}

async function mount() {
  const { bridge, choose } = fakeBridge();
  setMenuBridge(bridge);
  render(<DesktopMenuRoot />);
  await act(async () => {});
  return { bridge, choose };
}

afterEach(() => {
  setMenuBridge(undefined);
  setPaletteSource(null);
  act(() => useKeymapStore.setState({ keymap: DEFAULT_KEYMAP }));
  document.body.innerHTML = "";
});

describe("desktop menu bar language", () => {
  afterEach(() => resetI18nForTests());

  it("sends the labels at start and again on every language switch", async () => {
    const { bridge } = await mount();
    expect(bridge.setLabels).toHaveBeenLastCalledWith(
      expect.objectContaining({
        quit: "quit ethereal",
        file: "file",
        actions: expect.objectContaining({ "project:save-as": "save as…", "edit.undo": "undo" }),
      }),
    );
    await act(() => setLocale("fr", { persist: false }));
    expect(bridge.setLabels).toHaveBeenLastCalledWith(
      expect.objectContaining({
        quit: "quitter ethereal",
        file: "fichier",
        actions: expect.objectContaining({ "project:save-as": "enregistrer sous…", "edit.undo": "annuler" }),
      }),
    );
  });
});

describe("desktop menu bar", () => {
  it("sends the keyed items' chords, and again after a rebind", async () => {
    const { bridge } = await mount();
    expect(bridge.setShortcuts).toHaveBeenLastCalledWith([
      { action: "edit.undo", chord: "Mod+Z" },
      { action: "edit.redo", chord: "Mod+Shift+Z" },
      { action: "edit.selectAll", chord: "Mod+A" },
    ]);
    act(() => useKeymapStore.setState({ keymap: withBinding(DEFAULT_KEYMAP, "edit.undo", ["Mod+U"]) }));
    expect(bridge.setShortcuts).toHaveBeenLastCalledWith(expect.arrayContaining([{ action: "edit.undo", chord: "Mod+U" }]));
    act(() => useKeymapStore.setState({ keymap: withBinding(DEFAULT_KEYMAP, "edit.undo", []) }));
    expect(bridge.setShortcuts).toHaveBeenLastCalledWith(expect.arrayContaining([{ action: "edit.undo", chord: null }]));
  });

  it("Edit → undo runs the keymap's undo (as ⌘Z would); in a text field it is the field's own", async () => {
    const { bridge, choose } = await mount();
    const undo = vi.fn();
    const onKey = (e: KeyboardEvent) => matchesAction("edit.undo", e) && undo();
    window.addEventListener("keydown", onKey);
    try {
      choose("edit.undo");
      expect(undo).toHaveBeenCalledOnce();
      expect(bridge.nativeEdit).not.toHaveBeenCalled();
      const input = document.createElement("input");
      document.body.append(input);
      input.focus();
      choose("edit.undo");
      expect(bridge.nativeEdit).toHaveBeenCalledWith("edit.undo");
      expect(undo).toHaveBeenCalledOnce();
    } finally {
      window.removeEventListener("keydown", onKey);
    }
  });

  it("select all runs in the focused editor; Help items run their palette command", async () => {
    const { choose } = await mount();
    const editor = document.createElement("div");
    editor.tabIndex = 0;
    editor.dataset.keymapScope = "arrangement";
    const selectAll = vi.fn();
    editor.addEventListener("keydown", (e) => matchesAction("edit.selectAll", e) && selectAll());
    document.body.append(editor);
    choose("edit.selectAll");
    expect(selectAll).toHaveBeenCalledOnce();
    const shortcuts = vi.fn();
    setPaletteSource(() => [{ id: "keymap:open", label: "Keyboard shortcuts…", group: "Appearance", run: shortcuts }]);
    choose("keymap:open");
    expect(shortcuts).toHaveBeenCalledOnce();
  });

  it("does nothing while the keymap editor records a chord", async () => {
    const { bridge, choose } = await mount();
    const undo = vi.fn();
    const onKey = (e: KeyboardEvent) => matchesAction("edit.undo", e) && undo();
    window.addEventListener("keydown", onKey);
    const resume = pauseDispatch();
    try {
      choose("edit.undo");
      expect(undo).not.toHaveBeenCalled();
      expect(bridge.nativeEdit).not.toHaveBeenCalled();
    } finally {
      resume();
      window.removeEventListener("keydown", onKey);
    }
  });

  it("the web build has no bridge: nothing is mounted or sent", async () => {
    setMenuBridge(null);
    expect(() => render(<DesktopMenuRoot />)).not.toThrow();
  });
});
