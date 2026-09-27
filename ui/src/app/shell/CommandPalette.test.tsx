import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { useArrangementUi } from "@/features/arrangement/state";
import { renderWithMock, resetStores } from "@/features/transport-bar/testUtils";
import { devicesOfTrack, tracksOrdered, useProjectStore, useSelectionStore } from "@/state";
import { getTheme, setTheme } from "@/theme";
import { CommandPalette } from "./CommandPalette";
import { fuzzyScore } from "./commands";
import { resetShell, useShellStore } from "./shellStore";

afterEach(() => {
  resetStores();
  resetShell();
  useSelectionStore.getState().selectTrack(null);
  useArrangementUi.getState().setTrackFocus(null);
  try {
    localStorage.clear();
  } catch {
    /* no storage */
  }
});

const project = () => useProjectStore.getState().project!;
const openPalette = () => fireEvent.keyDown(window, { key: "k", metaKey: true });
const search = () => screen.getByRole("combobox", { name: "Search commands" });
const options = () => within(screen.getByRole("listbox", { name: "Commands" })).queryAllByRole("option");

describe("fuzzyScore", () => {
  it("matches in-order characters and prefers substrings and word starts", () => {
    expect(fuzzyScore("amt", "Add MIDI track")).toBeGreaterThan(0);
    expect(fuzzyScore("xyz", "Add MIDI track")).toBe(-1);
    expect(fuzzyScore("midi", "Add MIDI track")).toBeGreaterThan(fuzzyScore("amt", "Add MIDI track"));
    expect(fuzzyScore("add", "Add audio track")).toBeGreaterThan(fuzzyScore("add", "Go to track Padded"));
  });
});

describe("CommandPalette", () => {
  it("opens with cmd+K, filters, and runs the chosen command with Enter", async () => {
    await renderWithMock(<CommandPalette />);
    const before = tracksOrdered(project()).length;
    act(openPalette);
    expect(screen.getByRole("dialog", { name: "Command palette" })).toBeInTheDocument();
    expect(search()).toHaveFocus();
    fireEvent.change(search(), { target: { value: "add midi" } });
    expect(options()[0]).toHaveTextContent("Add MIDI track");
    fireEvent.keyDown(search(), { key: "Enter" });
    await waitFor(() => expect(tracksOrdered(project())).toHaveLength(before + 1));
    expect(useShellStore.getState().paletteOpen).toBe(false);
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Command palette" })).toBeNull());
  });

  it("doesn't open while typing in a text field; Escape closes", async () => {
    await renderWithMock(
      <>
        <input aria-label="Name" />
        <CommandPalette />
      </>,
    );
    fireEvent.keyDown(screen.getByRole("textbox", { name: "Name" }), { key: "k", metaKey: true });
    expect(useShellStore.getState().paletteOpen).toBe(false);
    act(openPalette);
    fireEvent.keyDown(search(), { key: "Escape" });
    expect(useShellStore.getState().paletteOpen).toBe(false);
  });

  it("arrows move the active command; recent commands come first next time", async () => {
    await renderWithMock(<CommandPalette />);
    act(openPalette);
    fireEvent.change(search(), { target: { value: "theme" } });
    const theme = getTheme();
    fireEvent.click(options()[0]!);
    expect(getTheme()).not.toBe(theme);
    setTheme(theme);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

    act(openPalette);
    expect(options()[0]).toHaveTextContent(/theme/i);
    expect(options()[0]).toHaveAttribute("aria-selected", "true");
    fireEvent.keyDown(search(), { key: "ArrowDown" });
    expect(options()[1]).toHaveAttribute("aria-selected", "true");
    fireEvent.keyDown(search(), { key: "ArrowUp" });
    fireEvent.keyDown(search(), { key: "ArrowUp" });
    expect(options()[options().length - 1]).toHaveAttribute("aria-selected", "true");
  });

  it("goes to a track, and adds a device to the selected track", async () => {
    await renderWithMock(<CommandPalette />);
    const bass = tracksOrdered(project()).find((t) => t.name === "Bass")!;
    act(openPalette);
    fireEvent.change(search(), { target: { value: "go to bass" } });
    fireEvent.keyDown(search(), { key: "Enter" });
    expect(useArrangementUi.getState().trackFocus).toBe(bass.id);
    expect(useSelectionStore.getState().selectedTrack).toBe(bass.id);

    const count = devicesOfTrack(project(), bass.id).length;
    act(openPalette);
    fireEvent.change(search(), { target: { value: "add compressor" } });
    await waitFor(() => expect(options()[0]).toHaveTextContent("Add Compressor"));
    fireEvent.keyDown(search(), { key: "Enter" });
    await waitFor(() => expect(devicesOfTrack(project(), bass.id)).toHaveLength(count + 1));
    expect(devicesOfTrack(project(), bass.id).at(-1)!.name).toBe("Compressor");
  });

  it("works without an engine (panel and theme commands only)", () => {
    render(<CommandPalette />);
    act(openPalette);
    fireEvent.change(search(), { target: { value: "open plugins" } });
    fireEvent.keyDown(search(), { key: "Enter" });
    expect(useShellStore.getState().left).toMatchObject({ open: true, tab: "plugins" });
  });
});
