import { afterEach, describe, expect, it } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { closeKeymapEditor, useKeymapStore } from "@/features/keymap";
import { ShortcutSettings } from "./InputSettings";
import { openSettings, useAudioSettings } from "./store";

afterEach(() => {
  closeKeymapEditor();
  useAudioSettings.getState().close();
});

describe("Settings > Input", () => {
  it("opens the keyboard shortcuts editor (#717: it was only in the palette)", () => {
    openSettings("shortcuts");
    render(<ShortcutSettings />);
    expect(screen.getByTestId("settings-shortcuts")).toHaveTextContent("keyboard shortcuts");
    fireEvent.click(screen.getByRole("button", { name: "edit keyboard shortcuts…" }));
    expect(useKeymapStore.getState().editorOpen).toBe(true);
    expect(useAudioSettings.getState().open).toBe(false);
  });
});
