import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { renderWithMock, resetStores } from "@/features/transport-bar/testUtils";
import { startOf } from "@/features/arrangement/clipTime";
import { arrangementView, resetArrangementUi, useArrangementUi } from "@/features/arrangement/uiStore";
import { useEditorStore, useProjectStore } from "@/state";
import { size as tokenSize } from "@/theme";
import { itemSelection } from "@/timeline";
import { App } from "./App";
import { resetShell, useShellStore } from "./shell/shellStore";

const workspace = () => document.querySelector<HTMLElement>(".eth-workspace")!;
const pane = (side: "left" | "right" | "bottom") => document.querySelector<HTMLElement>(`[data-pane="${side}"]`);
const clipEl = (name: string) => {
  const clip = Object.values(useProjectStore.getState().project!.clips).find((c) => c.name === name)!;
  return document.querySelector<HTMLElement>(`[data-clip-id="${clip.id}"]`)!;
};

beforeEach(() => {
  resetShell();
  resetArrangementUi();
  // jsdom has no canvas: clip waveforms just skip drawing.
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
});

afterEach(() => {
  cleanup();
  resetStores();
  itemSelection.getState().clear();
  useEditorStore.setState({ clip: null, request: 0 });
  vi.restoreAllMocks();
});

describe("App shell", () => {
  it("mounts the top bar slots and the arrangement as the main view", () => {
    const { container } = render(<App />);
    for (const slot of ["project", "transport-bar", "metronome", "recording", "export", "remote", "collab", "markers", "main"]) {
      expect(container.querySelector(`[data-slot="${slot}"]`), slot).not.toBeNull();
    }
    expect(container.querySelector('[data-slot="main"] [data-feature="arrangement"]')).not.toBeNull();
    expect(container.querySelector('[data-feature="mixer"]')).toBeNull(); // the mixer view is gone
  });

  it("has exactly one session element in the top bar: the Share button (base-115)", async () => {
    await renderWithMock(<App />);
    const top = document.querySelector<HTMLElement>('[data-slot="top"]')!;
    await waitFor(() => expect(top.querySelector('[data-testid="share-button"]')).not.toBeNull());
    // No relay "Collab" button, no peer chips, no idle "Remote" button.
    for (const id of ["collab-button", "collab-peers", "remote-button", "session-pill"]) {
      expect(top.querySelector(`[data-testid="${id}"]`), id).toBeNull();
    }
    // The settings (Audio | Sharing | Advanced) open from the gear.
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    expect(await screen.findByRole("tab", { name: "Sharing" })).toBeInTheDocument();
  });

  it("the rail opens each panel in the left pane, and closes it on a second click", async () => {
    const { container } = render(<App />);
    expect(pane("left")).toBeNull();
    for (const [label, feature] of [
      ["Library", "browser"],
      ["Plugins", "plugins"],
      ["MIDI mapping", "midi-learn"],
    ] as const) {
      fireEvent.click(screen.getByRole("button", { name: label }));
      expect(pane("left")).toHaveAttribute("aria-label", label);
      expect(container.querySelector(`[data-pane="left"] [data-feature="${feature}"]`), feature).not.toBeNull();
    }
    fireEvent.click(screen.getByRole("button", { name: "MIDI mapping" }));
    await waitFor(() => expect(pane("left")).toBeNull());
  });

  it("pinning a pane reserves its space next to the arrangement (flush: one gap, to the arrangement)", () => {
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Library" }));
    expect(workspace().style.getPropertyValue("--pane-left-reserved")).toBe("0px");
    fireEvent.click(screen.getByRole("button", { name: "Pin Library" }));
    const { size } = useShellStore.getState().left;
    expect(workspace().style.getPropertyValue("--pane-left-reserved")).toBe(`${size + parseFloat(tokenSize.floatGap)}px`);
    expect(pane("left")).toHaveClass("eth-float--pinned");
    fireEvent.click(screen.getByRole("button", { name: "Unpin Library" }));
    expect(workspace().style.getPropertyValue("--pane-left-reserved")).toBe("0px");
  });

  it("resizes a pane from its inner edge", () => {
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Library" }));
    const start = useShellStore.getState().left.size;
    const edge = screen.getByRole("separator", { name: "Resize Library" });
    fireEvent.pointerDown(edge, { button: 0, pointerId: 1, clientX: 300 });
    fireEvent.pointerMove(edge, { pointerId: 1, clientX: 340 });
    fireEvent.pointerUp(edge, { pointerId: 1, clientX: 340 });
    expect(useShellStore.getState().left.size).toBe(start + 40);
    expect(workspace().style.getPropertyValue("--pane-left-size")).toBe(`${start + 40}px`);
  });

  it("⌘J toggles the editor drawer; its tabs switch editors; Escape closes it", async () => {
    const { container } = render(<App />);
    expect(pane("bottom")).toBeNull();
    fireEvent.keyDown(window, { key: "j", metaKey: true });
    expect(pane("bottom")).not.toBeNull();
    for (const [tab, feature] of [
      ["Tempo", "tempo"],
      ["Groove", "groove"],
      ["Drum Rack", "drum-rack"],
    ] as const) {
      fireEvent.click(screen.getByRole("tab", { name: tab }));
      expect(screen.getByRole("tab", { name: tab })).toHaveAttribute("aria-selected", "true");
      expect(container.querySelector(`[data-pane="bottom"] [data-feature="${feature}"]`), feature).not.toBeNull();
    }
    fireEvent.keyDown(container.querySelector('[data-pane="bottom"] [data-slot="detail"]')!, { key: "Escape" });
    await waitFor(() => expect(pane("bottom")).toBeNull());
    fireEvent.keyDown(window, { key: "j", ctrlKey: true });
    expect(pane("bottom")).not.toBeNull();
    fireEvent.keyDown(window, { key: "j", ctrlKey: true });
    await waitFor(() => expect(pane("bottom")).toBeNull());
  });
});

describe("App shell: connected", () => {
  it("double-clicking a MIDI clip opens the drawer on the piano roll", async () => {
    const { container, mock } = await renderWithMock(<App />);
    expect(container.querySelectorAll('[data-slot="automation"] [data-automation-track]').length).toBeGreaterThan(0);
    await act(async () => {
      fireEvent.doubleClick(clipEl("Chords"));
    });
    expect(screen.getByRole("tab", { name: "Piano Roll" })).toHaveAttribute("aria-selected", "true");
    expect(container.querySelector('[data-pane="bottom"] [data-testid="piano-roll"]')).not.toBeNull();
    mock.dispose();
  });

  it("a click on a MIDI clip opens the piano roll; a click elsewhere in the tracks closes it unless pinned", async () => {
    const { mock } = await renderWithMock(<App />);
    act(() => arrangementView.getState().setViewport({ pxPerBeat: 10, scrollBeats: 0 }));
    const chords = Object.values(useProjectStore.getState().project!.clips).find((c) => c.name === "Chords")!;
    // jsdom lays nothing out: the content box is at 0,0 and Keys (the first row) is at y 0.
    const at = { button: 0, clientX: useArrangementUi.getState().headerWidth + (startOf(chords) + 0.5) * 10, clientY: 5 };
    const press = async (el: Element, p = at) => {
      await act(async () => {
        fireEvent.pointerDown(el, p);
        fireEvent.pointerUp(window, p);
      });
    };
    await press(clipEl("Chords"));
    expect(pane("bottom")).not.toBeNull();
    expect(useShellStore.getState().bottom.tab).toBe("piano-roll");
    expect(useEditorStore.getState().clip).toBe(chords.id);

    // A track header (outside any clip) closes it.
    const header = screen.getByRole("group", { name: "Keys track" });
    await press(header, { button: 0, clientX: 5, clientY: 5 });
    await waitFor(() => expect(pane("bottom")).toBeNull());

    // Pinned, it stays.
    await press(clipEl("Chords"));
    act(() => useShellStore.getState().setPinned("bottom", true));
    await press(header, { button: 0, clientX: 5, clientY: 5 });
    expect(pane("bottom")).not.toBeNull();
    mock.dispose();
  });

  it("a press anywhere in the arrangement collapses an unpinned left pane, not a pinned one", async () => {
    const { container, mock } = await renderWithMock(<App />);
    const arrangement = container.querySelector('[data-feature="arrangement"]')!;
    const markers = container.querySelector('[data-slot="markers"]')!;
    const header = screen.getByRole("group", { name: "Keys track" });
    const press = async (el: Element) => {
      await act(async () => {
        fireEvent.pointerDown(el, { button: 0, clientX: 5, clientY: 5 });
        fireEvent.pointerUp(window, { clientX: 5, clientY: 5 });
      });
    };
    for (const el of [arrangement, markers, header]) {
      act(() => useShellStore.getState().toggleLeft("library"));
      expect(pane("left")).not.toBeNull();
      await press(el);
      expect(useShellStore.getState().left.open).toBe(false);
      await waitFor(() => expect(pane("left")).toBeNull());
    }
    // The press still does its own job: a click on a track header selects the track.
    act(() => useShellStore.getState().toggleLeft("library"));
    await press(header);
    fireEvent.click(header);
    expect(useShellStore.getState().left.open).toBe(false);
    expect(useArrangementUi.getState().trackFocus).not.toBeNull();

    // Pinned, it stays open.
    act(() => useShellStore.getState().toggleLeft("library"));
    act(() => useShellStore.getState().setPinned("left", true));
    await press(arrangement);
    await press(header);
    expect(useShellStore.getState().left.open).toBe(true);
    expect(pane("left")).not.toBeNull();
    // A press inside the pane itself never collapses it.
    act(() => useShellStore.getState().setPinned("left", false));
    await press(pane("left")!);
    expect(useShellStore.getState().left.open).toBe(true);
    mock.dispose();
  });

  it("Escape closes an unpinned piano roll, from the arrangement or anywhere else", async () => {
    const { container, mock } = await renderWithMock(<App />);
    const arrangement = container.querySelector('[data-feature="arrangement"]')!;
    const open = () => act(() => useShellStore.getState().openDrawer("piano-roll"));
    open();
    fireEvent.keyDown(arrangement, { key: "Escape" });
    await waitFor(() => expect(pane("bottom")).toBeNull());
    open();
    fireEvent.keyDown(document.body, { key: "Escape" });
    await waitFor(() => expect(pane("bottom")).toBeNull());
    // Pinned, it stays.
    open();
    act(() => useShellStore.getState().setPinned("bottom", true));
    fireEvent.keyDown(arrangement, { key: "Escape" });
    fireEvent.keyDown(document.body, { key: "Escape" });
    expect(pane("bottom")).not.toBeNull();
    mock.dispose();
  });

  it("the inspector appears with a selected clip or track and hides when nothing is selected", async () => {
    const { container, mock } = await renderWithMock(<App />);
    expect(pane("right")).toBeNull();
    await act(async () => {
      fireEvent.pointerDown(clipEl("Chords"), { button: 0, clientX: 10, clientY: 5 });
      fireEvent.pointerUp(window, { clientX: 10, clientY: 5 });
    });
    expect(pane("right")).toHaveAttribute("aria-label", "Inspector");

    act(() => {
      fireEvent.keyDown(container.querySelector('[data-feature="arrangement"]')!, { key: "Escape" });
    });
    await waitFor(() => expect(pane("right")).toBeNull());

    fireEvent.click(screen.getByRole("group", { name: "Keys track" }));
    expect(pane("right")).not.toBeNull();
    act(() => {
      fireEvent.keyDown(container.querySelector('[data-feature="arrangement"]')!, { key: "Escape" });
    });
    await waitFor(() => expect(pane("right")).toBeNull());
    mock.dispose();
  });
});
