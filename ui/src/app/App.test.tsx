import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { renderWithMock, resetStores } from "@/features/transport-bar/testUtils";
import { resetArrangementUi } from "@/features/arrangement/uiStore";
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

  it("pinning a pane reserves its space next to the arrangement", () => {
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Library" }));
    expect(workspace().style.getPropertyValue("--pane-left-reserved")).toBe("0px");
    fireEvent.click(screen.getByRole("button", { name: "Pin Library" }));
    const { size } = useShellStore.getState().left;
    expect(workspace().style.getPropertyValue("--pane-left-reserved")).toBe(`${size + 2 * parseFloat(tokenSize.floatGap)}px`);
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
