import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { LibraryItem, Project } from "@/generated";
import { useArrangementUi } from "@/features/arrangement/state";
import { useBrowserReveal } from "@/features/browser/reveal";
import { useDeviceClipboard } from "@/features/devices/deviceClipboard";
import { useTimeSelection } from "@/features/time-edits/store";
import { renderWithMock, resetStores } from "@/features/transport-bar/testUtils";
import { resetI18nForTests } from "@/i18n/runtime";
import { createDemoProject } from "@/transport/mock/demoProject";
import { tracksOrdered, useProjectStore, useSelectionStore } from "@/state";
import { resetShell, useShellStore } from "@/state/shell";
import { itemSelection } from "@/timeline";
import { QuickSearch } from "./QuickSearch";
import { goTo, libraryResults, searchProject, useQuickSearch } from "./quickSearchIndex";

afterEach(() => {
  resetI18nForTests();
  resetStores();
  resetShell();
  useQuickSearch.setState({ open: false });
  useBrowserReveal.setState({ request: null });
  useDeviceClipboard.getState().select(null);
  useSelectionStore.getState().selectTrack(null);
  useArrangementUi.getState().setTrackFocus(null);
});

function withMarker(p: Project): Project {
  return { ...p, markers: { m1: { id: "m1", position: 32, name: "Chorus", color: null } } };
}

const openSearch = () => fireEvent.keyDown(window, { key: "f", metaKey: true });
const input = () => screen.getByRole("combobox", { name: "search the project and the library" });
const options = () => within(screen.getByRole("listbox", { name: "results" })).queryAllByRole("option");

describe("searchProject", () => {
  it("finds tracks, devices, clips and markers by name, best first", () => {
    const p = withMarker(createDemoProject());
    expect(searchProject(p, "")).toEqual([]);
    const bass = searchProject(p, "bass");
    expect(bass[0]).toMatchObject({ type: "track", name: "Bass" });
    // The track's clips (named after it) come after it.
    expect(bass.some((r) => r.type === "clip" && r.detail === "Bass")).toBe(true);
    expect(searchProject(p, "compressor")[0]).toMatchObject({ type: "device", name: "Compressor" });
    expect(searchProject(p, "chorus")[0]).toMatchObject({ type: "marker", name: "Chorus" });
    expect(searchProject(p, "zzzz")).toEqual([]);
  });

  it("names library items with their folder", () => {
    const item = { id: "lib/Drums/Kick.wav", kind: "Audio", name: "Kick", root: "lib", path: "Drums/Kick.wav" } as LibraryItem;
    expect(libraryResults([item])[0]).toMatchObject({ type: "library", name: "Kick", detail: "Drums" });
  });

  it("reveals a library item in the browser", () => {
    const item = { id: "lib/Drums/Kick.wav", kind: "Audio", name: "Kick", root: "lib", path: "Drums/Kick.wav" } as LibraryItem;
    goTo(libraryResults([item])[0]!, null);
    expect(useShellStore.getState().left).toMatchObject({ open: true, tab: "library" });
    expect(useBrowserReveal.getState().request?.item.path).toBe("Drums/Kick.wav");
  });
});

describe("QuickSearch", () => {
  it("opens with cmd+F, also from a text field, and Enter selects a track", async () => {
    await renderWithMock(
      <>
        <input aria-label="Name" />
        <QuickSearch />
      </>,
    );
    screen.getByLabelText("Name").focus();
    const ev = new KeyboardEvent("keydown", { key: "f", metaKey: true, bubbles: true, cancelable: true });
    act(() => void screen.getByLabelText("Name").dispatchEvent(ev));
    expect(ev.defaultPrevented).toBe(true);
    expect(screen.getByRole("dialog", { name: "quick search" })).toBeInTheDocument();
    expect(input()).toHaveFocus();
    const bass = tracksOrdered(useProjectStore.getState().project!).find((t) => t.name === "Bass")!;
    fireEvent.change(input(), { target: { value: "bass" } });
    expect(options()[0]).toHaveTextContent("Bass");
    fireEvent.keyDown(input(), { key: "Enter" });
    expect(useArrangementUi.getState().trackFocus).toBe(bass.id);
    expect(useQuickSearch.getState().open).toBe(false);
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "quick search" })).toBeNull());
  });

  it("jumps to a device, a clip and a marker", async () => {
    await renderWithMock(<QuickSearch />);
    act(() => useProjectStore.setState({ project: withMarker(useProjectStore.getState().project!) }));
    const project = useProjectStore.getState().project!;

    act(openSearch);
    fireEvent.change(input(), { target: { value: "compressor" } });
    fireEvent.keyDown(input(), { key: "Enter" });
    const comp = Object.values(project.devices).find((d) => d.name === "Compressor")!;
    expect(useDeviceClipboard.getState().selected).toEqual([comp.id]);
    expect(useSelectionStore.getState().selectedTrack).toBe(comp.track);

    act(openSearch);
    fireEvent.change(input(), { target: { value: "chorus" } });
    fireEvent.keyDown(input(), { key: "Enter" });
    expect(useTimeSelection.getState().cursor).toBe(32);

    const clip = Object.values(project.clips)[0]!;
    const name = clip.name || project.tracks[clip.track]!.name;
    act(openSearch);
    fireEvent.change(input(), { target: { value: name } });
    const row = options().find((o) => o.dataset.kind === "clip")!;
    fireEvent.click(row);
    expect(itemSelection.getState().selected.clip.has(clip.id)).toBe(true);
  });

  it("Escape closes, cmd+F toggles", async () => {
    await renderWithMock(<QuickSearch />);
    act(openSearch);
    expect(screen.getByText("type a name: tracks, clips, devices, markers and library items")).toBeInTheDocument();
    fireEvent.keyDown(input(), { key: "Escape" });
    expect(useQuickSearch.getState().open).toBe(false);
    act(openSearch);
    expect(useQuickSearch.getState().open).toBe(true);
    act(openSearch);
    expect(useQuickSearch.getState().open).toBe(false);
  });
});
