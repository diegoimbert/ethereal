import { act, within, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { renderWithMock, resetStores } from "@/features/transport-bar/testUtils";
import { startOf } from "@/features/arrangement/clipTime";
import { arrangementView, resetArrangementUi, useArrangementUi } from "@/features/arrangement/uiStore";
import { useEditorStore, useProjectStore } from "@/state";
import { size as tokenSize } from "@/theme";
import { itemSelection } from "@/timeline";
import { App } from "./App";
import { buildCommands } from "./shell/commands";
import { clipMenu } from "@/features/arrangement/actions";
import { resetShell, useShellStore } from "@/state/shell";
import { useAudioSettings } from "@/features/audio-settings/store";

const workspace = () => document.querySelector<HTMLElement>(".eth-workspace")!;
const pane = (side: "left" | "right" | "bottom") => document.querySelector<HTMLElement>(`[data-pane="${side}"]`);
/** The arranger's lanes (the canvas hit-tests presses on them). */
const lanes = () => screen.getByTestId("arrangement-content");
/** The middle of a clip's title band in client px (the arranger test API; jsdom lays out nothing). */
const clipTitle = (name: string) => {
  const clip = Object.values(useProjectStore.getState().project!.clips).find((c) => c.name === name)!;
  const api = (
    window as unknown as { __ether: { arrangement: { handleRect(id: string, part: string): { x: number; y: number; width: number; height: number } | null } } }
  ).__ether.arrangement;
  const r = api.handleRect(clip.id, "header")!;
  return { clientX: r.x + r.width / 2, clientY: r.y + r.height / 2 };
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
  // A dialog left open (the settings) would take the next test's Escape (#578).
  useAudioSettings.getState().close();
  vi.restoreAllMocks();
});

describe("App shell", () => {
  it("mounts the top bar slots and the arrangement as the main view", () => {
    const { container } = render(<App />);
    for (const slot of ["project", "transport-bar", "metronome", "recording", "export", "remote", "markers", "main"]) {
      expect(container.querySelector(`[data-slot="${slot}"]`), slot).not.toBeNull();
    }
    expect(container.querySelector('[data-slot="main"] [data-feature="arrangement"]')).not.toBeNull();
    expect(container.querySelector('[data-feature="mixer"]')).toBeNull(); // the mixer view is gone
  });

  it("has no collaboration session controls in the top bar (base-115)", async () => {
    await renderWithMock(<App />);
    const top = document.querySelector<HTMLElement>('[data-slot="top"]')!;
    // No Share button, relay "Collab" button, peer chips, idle "Remote" button or session pill.
    for (const id of ["share-button", "collab-button", "collab-peers", "remote-button", "session-pill"]) {
      expect(top.querySelector(`[data-testid="${id}"]`), id).toBeNull();
    }
    // The settings open from the gear.
    fireEvent.click(screen.getByRole("button", { name: "settings" }));
    expect(await screen.findByRole("tab", { name: "general" })).toBeInTheDocument();
  });

  it("the rail opens each panel in the left pane, and closes it on a second click", async () => {
    const { container } = render(<App />);
    expect(pane("left")).toBeNull();
    for (const [label, feature] of [
      ["library", "browser"],
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
    fireEvent.click(screen.getByRole("button", { name: "library" }));
    expect(workspace().style.getPropertyValue("--pane-left-reserved")).toBe("0px");
    fireEvent.click(screen.getByRole("button", { name: "pin the library panel" }));
    const { size } = useShellStore.getState().left;
    expect(workspace().style.getPropertyValue("--pane-left-reserved")).toBe(`${size + parseFloat(tokenSize.floatGap)}px`);
    expect(pane("left")).toHaveClass("eth-float--pinned");
    fireEvent.click(screen.getByRole("button", { name: "unpin the library panel" }));
    expect(workspace().style.getPropertyValue("--pane-left-reserved")).toBe("0px");
  });

  it("resizes a pane from its inner edge", () => {
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "library" }));
    const start = useShellStore.getState().left.size;
    const edge = screen.getByRole("separator", { name: "resize the library panel" });
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
    // The editors that apply: no clip open, only Automation (the piano roll is for a MIDI clip,
    // Warp for an audio one; tempo is on the master track, groove in the clip inspector, drum
    // pads in the Drum Rack device).
    expect(
      within(screen.getByRole("tablist", { name: "editors" }))
        .getAllByRole("tab")
        .map((t) => t.textContent),
    ).toEqual(["automation"]);
    expect(screen.getByRole("tab", { name: "automation" })).toHaveAttribute("aria-selected", "true");
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
      fireEvent.doubleClick(lanes(), clipTitle("Chords"));
    });
    expect(screen.getByRole("tab", { name: "piano roll" })).toHaveAttribute("aria-selected", "true");
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
    await press(lanes());
    expect(pane("bottom")).not.toBeNull();
    expect(useShellStore.getState().bottom.tab).toBe("piano-roll");
    expect(useEditorStore.getState().clip).toBe(chords.id);

    // A track header (outside any clip) closes it.
    const header = screen.getByRole("group", { name: "Keys track" });
    await press(header, { button: 0, clientX: 5, clientY: 5 });
    await waitFor(() => expect(pane("bottom")).toBeNull());

    // Pinned, it stays.
    await press(lanes());
    act(() => useShellStore.getState().setPinned("bottom", true));
    await press(header, { button: 0, clientX: 5, clientY: 5 });
    expect(pane("bottom")).not.toBeNull();
    mock.dispose();
  });

  describe("the drawer opens and closes with clicks", () => {
    const click = async (el: Element, p: { clientX: number; clientY: number }, to = p, extra = {}) => {
      await act(async () => {
        fireEvent.pointerDown(el, { button: 0, ...p, ...extra });
        fireEvent.pointerUp(window, { button: 0, ...to });
      });
    };

    it("a click on an audio clip opens its editor; a drag, a modifier click and a right-click do not", async () => {
      const { mock } = await renderWithMock(<App />);
      const drum = Object.values(useProjectStore.getState().project!.clips).find((c) => c.name === "drum-loop-120")!;
      const at = clipTitle("drum-loop-120");
      await click(lanes(), at, { clientX: at.clientX + 40, clientY: at.clientY });
      expect(pane("bottom")).toBeNull();
      await click(lanes(), at, at, { shiftKey: true });
      await click(lanes(), at, at, { metaKey: true });
      expect(pane("bottom")).toBeNull();
      await act(async () => {
        fireEvent.pointerDown(lanes(), { button: 2, ...at });
        fireEvent.pointerUp(window, at);
      });
      expect(pane("bottom")).toBeNull();
      await click(lanes(), at);
      expect(pane("bottom")).not.toBeNull();
      expect(useShellStore.getState().bottom.tab).toBe("warp");
      expect(useEditorStore.getState().clip).toBe(drum.id);
      mock.dispose();
    });

    it("a click away closes an unpinned drawer, any tab; pinned it stays", async () => {
      const { mock } = await renderWithMock(<App />);
      const away = [lanes(), document.querySelector('[data-slot="markers"]')!, document.body];
      for (const el of away) {
        act(() => useShellStore.getState().openDrawer("warp"));
        await click(el, { clientX: 5, clientY: 5 });
        await waitFor(() => expect(pane("bottom")).toBeNull());
      }
      act(() => useShellStore.getState().openDrawer("warp"));
      act(() => useShellStore.getState().setPinned("bottom", true));
      for (const el of away) await click(el, { clientX: 5, clientY: 5 });
      expect(pane("bottom")).not.toBeNull();
      mock.dispose();
    });

    it("presses inside the drawer, its resize handle or a floating layer do not close it", async () => {
      const { mock } = await renderWithMock(<App />);
      act(() => useShellStore.getState().openDrawer("warp"));
      const p = { clientX: 5, clientY: 5 };
      await click(pane("bottom")!, p);
      await click(screen.getByRole("separator", { name: "resize the editor panel" }), p);
      const layer = document.createElement("div");
      layer.className = "eth-popover";
      document.body.appendChild(layer);
      await click(layer, p);
      layer.remove();
      // The tools used while editing: the top bar (transport included), the rail and the side panes.
      await click(document.querySelector(".eth-rail")!, p);
      await click(document.querySelector('[data-slot="transport-bar"]')!, p);
      act(() => useShellStore.getState().setOpen("left", true));
      await click(document.querySelector('[data-pane="left"]')!, p);
      expect(pane("bottom")).not.toBeNull();
      mock.dispose();
    });
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

  it("the palette's editor actions run in the arrangement through its key handler (#577)", async () => {
    const { mock } = await renderWithMock(<App />);
    const clips = () => Object.values(useProjectStore.getState().project!.clips);
    const chords = clips().find((c) => c.name === "Chords")!;
    act(() => itemSelection.getState().select("clip", [chords.id], "replace"));
    const before = clips().length;
    const duplicate = buildCommands(mock, []).find((c) => c.id === "editor:edit.duplicate")!;
    expect(duplicate.label).toBe("duplicate");
    act(() => duplicate.run());
    await waitFor(() => expect(clips().length).toBe(before + 1));
    // The copy is selected: the palette's delete removes it.
    act(() =>
      buildCommands(mock, [])
        .find((c) => c.id === "editor:edit.delete")!
        .run(),
    );
    await waitFor(() => expect(clips().length).toBe(before));
    expect(clips().some((c) => c.id === chords.id)).toBe(true);
    mock.dispose();
  });

  it("an audio clip's menu opens it in its editor (#575)", async () => {
    const { mock } = await renderWithMock(<App />);
    const drum = Object.values(useProjectStore.getState().project!.clips).find((c) => c.content.type === "Audio")!;
    const entry = clipMenu(mock, drum).find((i) => i !== "separator" && i.label === "open in editor");
    expect(entry).toBeDefined();
    act(() => (entry as Exclude<typeof entry, "separator" | undefined>).onSelect());
    expect(useEditorStore.getState().clip).toBe(drum.id);
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
      const at = clipTitle("Chords");
      fireEvent.pointerDown(lanes(), { button: 0, ...at });
      fireEvent.pointerUp(window, at);
    });
    expect(pane("right")).toHaveAttribute("aria-label", "inspector");

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

describe("App shell: collapsing the inspector and the bottom pane", () => {
  const selectKeys = () => fireEvent.click(screen.getByRole("group", { name: "Keys track" }));
  const stored = () => JSON.parse(localStorage.getItem("eth.shell") ?? "{}") as { right?: { collapsed?: boolean }; bottom?: { collapsed?: boolean } };

  it("the inspector's × collapses it to a bar; it reopens collapsed; expanding is remembered; Esc still closes", async () => {
    const { container, mock } = await renderWithMock(<App />);
    selectKeys();
    fireEvent.click(screen.getByRole("button", { name: "collapse the inspector panel" }));
    expect(useShellStore.getState().right.collapsed).toBe(true);
    expect(stored().right?.collapsed).toBe(true);
    // Still "open" (selection kept), but only the bar shows: the card and its content are unmounted.
    // The card flies out first (content mounted meanwhile), then the bar shows.
    expect(screen.getByTestId("inspector")).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByTestId("inspector")).toBeNull());
    const bar = await screen.findByRole("button", { name: "expand the inspector panel" });
    expect(bar).toHaveTextContent("inspector");
    expect(useShellStore.getState().right.open).toBe(true);
    expect(workspace().style.getPropertyValue("--pane-right-reserved")).toBe("0px");

    // Esc closes it (bar gone); selecting again reopens it collapsed.
    act(() => {
      fireEvent.keyDown(container.querySelector('[data-feature="arrangement"]')!, { key: "Escape" });
    });
    await waitFor(() => expect(screen.queryByRole("button", { name: "expand the inspector panel" })).toBeNull());
    selectKeys();
    expect(await screen.findByRole("button", { name: "expand the inspector panel" })).toBeInTheDocument();
    expect(screen.queryByTestId("inspector")).toBeNull();

    // Clicking the bar expands; the choice persists for the next time.
    fireEvent.click(screen.getByRole("button", { name: "expand the inspector panel" }));
    expect(screen.getByTestId("inspector")).toBeInTheDocument();
    expect(stored().right?.collapsed).toBe(false);
    mock.dispose();
  });

  it("the bar is a button (Enter expands), and Space on it plays / stops instead of expanding", async () => {
    const { mock } = await renderWithMock(<App />);
    selectKeys();
    fireEvent.click(screen.getByRole("button", { name: "collapse the inspector panel" }));
    const bar = await screen.findByRole("button", { name: "expand the inspector panel" });
    expect(bar.tagName).toBe("BUTTON");
    bar.focus();
    const space = new KeyboardEvent("keydown", { key: " ", code: "Space", bubbles: true, cancelable: true });
    bar.dispatchEvent(space);
    expect(space.defaultPrevented).toBe(true); // the transport took it (capture listener)
    expect(useShellStore.getState().right.collapsed).toBe(true);
    mock.dispose();
  });

  it("double-clicking a clip expands the collapsed drawer and inspector at their last size (#758)", async () => {
    const { container, mock } = await renderWithMock(<App />);
    act(() => {
      const s = useShellStore.getState();
      s.setSize("right", 333);
      s.setSize("bottom", 222);
      s.setCollapsed("right", true);
      s.setCollapsed("bottom", true);
    });
    // A click opens the clip; the panels stay collapsed (a remembered choice).
    fireEvent.pointerDown(lanes(), { button: 0, ...clipTitle("Chords") });
    fireEvent.pointerUp(window, { button: 0, ...clipTitle("Chords") });
    expect(await screen.findByRole("button", { name: "expand the editor panel" })).toBeInTheDocument();
    expect(await screen.findByRole("button", { name: "expand the inspector panel" })).toBeInTheDocument();
    // A double-click expands both, at their last size.
    await act(async () => {
      fireEvent.doubleClick(lanes(), clipTitle("Chords"));
    });
    expect(useShellStore.getState().right).toMatchObject({ open: true, collapsed: false, size: 333 });
    expect(useShellStore.getState().bottom).toMatchObject({ open: true, collapsed: false, size: 222, tab: "piano-roll" });
    expect(stored().right?.collapsed).toBe(false);
    expect(stored().bottom?.collapsed).toBe(false);
    expect(screen.getByTestId("inspector")).toBeInTheDocument();
    expect(container.querySelector('[data-pane="bottom"] [data-testid="piano-roll"]')).not.toBeNull();
    // They collapse again as usual.
    fireEvent.click(screen.getByRole("button", { name: "collapse the editor panel" }));
    expect(useShellStore.getState().bottom.collapsed).toBe(true);
    mock.dispose();
  });

  it("double-clicking the open clip with both panels expanded collapses them, and again expands them at the same sizes", async () => {
    const { mock } = await renderWithMock(<App />);
    // A real double-click: two presses (the second one's `detail` is 2), then the dblclick.
    const doubleClick = async (name: string) => {
      const at = clipTitle(name);
      await act(async () => {
        for (const detail of [1, 2]) {
          fireEvent.pointerDown(lanes(), { button: 0, detail, ...at });
          fireEvent.pointerUp(window, { button: 0, detail, ...at });
        }
        fireEvent.doubleClick(lanes(), at);
      });
    };
    act(() => {
      const s = useShellStore.getState();
      s.setSize("right", 333);
      s.setSize("bottom", 222);
    });
    const chords = Object.values(useProjectStore.getState().project!.clips).find((c) => c.name === "Chords")!;
    const bass = Object.values(useProjectStore.getState().project!.clips).find((c) => c.name === "Bassline")!;
    // Both collapsed: a double-click expands both.
    await doubleClick("Chords");
    expect(useShellStore.getState().right).toMatchObject({ open: true, collapsed: false, size: 333 });
    expect(useShellStore.getState().bottom).toMatchObject({ open: true, collapsed: false, size: 222 });
    // Both expanded, the same clip: both collapse, sizes kept; the clip stays selected and open.
    await doubleClick("Chords");
    expect(useShellStore.getState().right).toMatchObject({ collapsed: true, size: 333 });
    expect(useShellStore.getState().bottom).toMatchObject({ collapsed: true, size: 222 });
    expect(stored().right?.collapsed).toBe(true);
    expect(stored().bottom?.collapsed).toBe(true);
    expect(useEditorStore.getState().clip).toBe(chords.id);
    expect(itemSelection.getState().selected.clip.has(chords.id)).toBe(true);
    // Again: both expand back, at the same sizes.
    await doubleClick("Chords");
    expect(useShellStore.getState().right).toMatchObject({ open: true, collapsed: false, size: 333 });
    expect(useShellStore.getState().bottom).toMatchObject({ open: true, collapsed: false, size: 222 });
    // Both expanded, another clip: the editor switches to it and both stay open.
    await doubleClick("Bassline");
    expect(useEditorStore.getState().clip).toBe(bass.id);
    expect(useShellStore.getState().right).toMatchObject({ open: true, collapsed: false });
    expect(useShellStore.getState().bottom).toMatchObject({ open: true, collapsed: false });
    mock.dispose();
  });

  it("an open panel keeps its size on a double-click; a collapsed one too big for the window is clamped", async () => {
    const { mock } = await renderWithMock(<App />);
    const n = (token: string) => parseFloat(token);
    act(() => {
      const s = useShellStore.getState();
      s.setSize("right", 5000); // expanded: left as is, even past the window
      s.setSize("bottom", 5000);
      s.setCollapsed("bottom", true);
    });
    await act(async () => {
      fireEvent.doubleClick(lanes(), clipTitle("Chords"));
    });
    expect(useShellStore.getState().right).toMatchObject({ collapsed: false, size: 5000 });
    // The arrangement keeps its minimum height above the expanded drawer.
    const max = window.innerHeight - n(tokenSize.mainMinSize) - n(tokenSize.topBarHeight) - n(tokenSize.statusBarHeight);
    expect(useShellStore.getState().bottom).toMatchObject({ collapsed: false, size: Math.max(n(tokenSize.floatMinSize), max) });

    // An audio clip's body (below its title bar) expands them too, on the warp tab.
    act(() => useShellStore.getState().setCollapsed("bottom", true));
    const api = (
      window as unknown as { __ether: { arrangement: { handleRect(id: string, part: string): { x: number; y: number; width: number; height: number } } } }
    ).__ether.arrangement;
    const drum = Object.values(useProjectStore.getState().project!.clips).find((c) => c.name === "drum-loop-120")!;
    const box = api.handleRect(drum.id, "box");
    await act(async () => {
      fireEvent.doubleClick(lanes(), { clientX: box.x + box.width / 2, clientY: box.y + box.height - 4 });
    });
    expect(useEditorStore.getState().clip).toBe(drum.id);
    expect(useShellStore.getState().bottom).toMatchObject({ collapsed: false, tab: "warp" });
    mock.dispose();
  });

  it("the bottom pane collapses to a bar that is not a click-away; reopening keeps it collapsed", async () => {
    const { mock } = await renderWithMock(<App />);
    act(() => useShellStore.getState().openDrawer("warp"));
    fireEvent.click(screen.getByRole("button", { name: "collapse the editor panel" }));
    expect(stored().bottom?.collapsed).toBe(true);
    await waitFor(() => expect(document.querySelector(".eth-drawer")).toBeNull());
    const bar = await screen.findByRole("button", { name: "expand the editor panel" });
    expect(bar).toHaveTextContent("editor");
    // Pressing the bar does not count as a click-away.
    fireEvent.pointerDown(bar, { button: 0 });
    fireEvent.pointerUp(window, { button: 0 });
    expect(useShellStore.getState().bottom.open).toBe(true);
    // A click away still closes it; Ctrl+J reopens it collapsed.
    await act(async () => {
      fireEvent.pointerDown(document.body, { button: 0 });
      fireEvent.pointerUp(window, { button: 0 });
    });
    expect(useShellStore.getState().bottom.open).toBe(false);
    fireEvent.keyDown(window, { key: "j", ctrlKey: true });
    expect(await screen.findByRole("button", { name: "expand the editor panel" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "expand the editor panel" }));
    expect(document.querySelector(".eth-drawer")).not.toBeNull();
    expect(stored().bottom?.collapsed).toBe(false);
    mock.dispose();
  });
});
