/** The keymap editor and cheat sheet, against the MockTransport (which stores the keymap). */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { pickOption } from "@/kit/testing";
import { cmd } from "@/transport";
import { renderWithMock, resetStores } from "@/features/transport-bar/testUtils";
import { CheatSheetContent } from "./CheatSheet";
import { KeymapRoot } from "./KeymapRoot";
import { setPaletteSource } from "./palette";
import { usePrinting } from "./print";
import { DEFAULT_KEYMAP, matchesAction, openKeymapEditor, setPaletteActions, useKeymapStore, withBinding } from "./store";

afterEach(() => {
  resetStores();
  useKeymapStore.setState({ keymap: DEFAULT_KEYMAP, stored: false, error: null, editorOpen: false });
  setPaletteSource(null);
  setPaletteActions([]);
  usePrinting.setState({ printing: false });
  vi.restoreAllMocks();
});

const row = (id: string) => document.querySelector<HTMLElement>(`[data-action="${id}"]`)!;

async function setup() {
  const r = await renderWithMock(<KeymapRoot />);
  await waitFor(() => expect(useKeymapStore.getState().stored).toBe(true));
  act(() => openKeymapEditor());
  await screen.findByTestId("keymap-editor");
  return r;
}

async function getStored(mock: Awaited<ReturnType<typeof setup>>["mock"]) {
  const reply = await mock.send(cmd("Keymap", { type: "Get" }));
  return reply.type === "Keymap" ? reply.keymap : null;
}

describe("KeymapEditor", () => {
  it("lists every action with its chord, by group", async () => {
    await setup();
    expect(within(row("edit.duplicate")).getByText("Ctrl+D")).toBeInTheDocument();
    expect(within(row("transport.play")).getByText("Space")).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Time selection" })).toBeInTheDocument();
  });

  it("records a chord, stores it in the engine, and the lookup follows", async () => {
    const { mock } = await setup();
    fireEvent.click(within(row("transport.metronome")).getByRole("button", { name: /Add a shortcut/ }));
    expect(screen.getByTestId("keymap-recording")).toBeInTheDocument();
    // A lone modifier keeps waiting; the chord is shown, then confirmed with Enter.
    act(() => void window.dispatchEvent(new KeyboardEvent("keydown", { key: "Alt", altKey: true, code: "AltLeft" })));
    act(() => void window.dispatchEvent(new KeyboardEvent("keydown", { key: "∑", altKey: true, code: "KeyM" })));
    expect(within(screen.getByTestId("keymap-pending")).getByText("Alt+M")).toBeInTheDocument();
    act(() => void window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" })));
    await waitFor(async () => expect(await getStored(mock)).toEqual({ preset: "Ethereal", overrides: [{ action: "transport.metronome", chords: ["Alt+M"] }] }));
    expect(matchesAction("transport.metronome", { key: "m", altKey: true, metaKey: false, ctrlKey: false, shiftKey: false })).toBe(true);
    // Reset the row back to the preset.
    fireEvent.click(within(row("transport.metronome")).getByRole("button", { name: /Reset Metronome/ }));
    await waitFor(async () => expect(await getStored(mock)).toEqual(DEFAULT_KEYMAP));
  });

  it("shows a conflict while recording and on both rows, and can unbind the other", async () => {
    const { mock } = await setup();
    fireEvent.click(within(row("transport.loop")).getByRole("button", { name: /Add a shortcut/ }));
    act(() => void window.dispatchEvent(new KeyboardEvent("keydown", { key: "d", ctrlKey: true, code: "KeyD" })));
    expect(within(screen.getByTestId("keymap-pending")).getByRole("alert")).toHaveTextContent("Also Duplicate");
    fireEvent.click(within(screen.getByTestId("keymap-pending")).getByRole("button", { name: "Use anyway" }));
    await waitFor(() => expect(within(row("edit.duplicate")).getByRole("note")).toHaveTextContent("is also Loop on / off"));
    expect(within(row("transport.loop")).getByRole("note")).toHaveTextContent("is also Duplicate");
    fireEvent.click(within(row("transport.loop")).getByRole("button", { name: "Remove from Duplicate" }));
    await waitFor(async () =>
      expect(await getStored(mock)).toEqual({
        preset: "Ethereal",
        overrides: [
          { action: "edit.duplicate", chords: [] },
          { action: "transport.loop", chords: ["Mod+D"] },
        ],
      }),
    );
    expect(document.querySelector('[role="note"]')).toBeNull();
  });

  it("Escape cancels recording without closing the dialog", async () => {
    await setup();
    fireEvent.click(within(row("transport.loop")).getByRole("button", { name: /Add a shortcut/ }));
    act(() => void window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" })));
    expect(screen.queryByTestId("keymap-recording")).toBeNull();
    expect(useKeymapStore.getState().editorOpen).toBe(true);
  });

  it("switches presets and resets everything", async () => {
    const { mock } = await setup();
    pickOption(screen.getByRole("combobox", { name: "Keymap preset" }), "Ableton-like");
    await waitFor(() => expect(within(row("transport.record")).getByText("F9")).toBeInTheDocument());
    // Ableton-like keeps the defaults.
    expect(within(row("view.editor")).getByText("Ctrl+J")).toBeInTheDocument();
    expect(await getStored(mock)).toEqual({ preset: "AbletonLike", overrides: [] });
    fireEvent.click(screen.getByRole("button", { name: /Reset all/ }));
    await waitFor(async () => expect(await getStored(mock)).toEqual(DEFAULT_KEYMAP));
    expect(within(row("transport.record")).queryByText("F9")).toBeNull();
  });

  it("searches by label or key and filters conflicts", async () => {
    await setup();
    fireEvent.change(screen.getByRole("textbox", { name: "Search shortcuts" }), { target: { value: "quantize" } });
    expect(row("pianoRoll.quantize")).toBeTruthy();
    expect(row("edit.duplicate")).toBeNull();
    fireEvent.change(screen.getByRole("textbox", { name: "Search shortcuts" }), { target: { value: "" } });
    fireEvent.click(screen.getByRole("switch"));
    expect(screen.getByText("No matching action.")).toBeInTheDocument();
  });

  it("follows KeymapEvent::Changed from another window", async () => {
    const { mock } = await setup();
    await act(async () => void (await mock.send(cmd("Keymap", { type: "Set", keymap: withBinding(DEFAULT_KEYMAP, "edit.copy", ["F2"]) }))));
    await waitFor(() => expect(within(row("edit.copy")).getByText("F2")).toBeInTheDocument());
  });

  it("prints the cheat sheet", async () => {
    const print = vi.fn();
    vi.stubGlobal("print", print);
    await setup();
    fireEvent.click(screen.getByRole("button", { name: /Print cheat sheet/ }));
    await waitFor(() => expect(print).toHaveBeenCalled());
    const sheet = document.querySelector(".eth-keymap-print")!;
    expect(sheet).toHaveAttribute("data-theme", "light");
    expect(document.body).toHaveClass("eth-keymap-printing");
    expect(within(sheet as HTMLElement).getByText("Duplicate")).toBeInTheDocument();
    act(() => void window.dispatchEvent(new Event("afterprint")));
    await waitFor(() => expect(document.querySelector(".eth-keymap-print")).toBeNull());
    expect(document.body).not.toHaveClass("eth-keymap-printing");
    vi.unstubAllGlobals();
  });
});

describe("CheatSheetContent", () => {
  it("lists bound actions only, with the user's chords", () => {
    useKeymapStore.setState({ keymap: withBinding(DEFAULT_KEYMAP, "edit.duplicate", []) });
    render(<CheatSheetContent />);
    const sheet = screen.getByTestId("keymap-cheat-sheet");
    expect(within(sheet).queryByText("Duplicate")).toBeNull();
    expect(within(sheet).getByText("Undo")).toBeInTheDocument();
    expect(within(sheet).getByText("Ethereal preset, customized")).toBeInTheDocument();
  });
});
