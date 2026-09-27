import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { renderWithMock, resetStores } from "@/features/transport-bar/testUtils";
import { useEditorStore, useProjectStore } from "@/state";
import { App } from "./App";

describe("App shell: connected", () => {
  afterEach(() => {
    cleanup();
    resetStores();
    useEditorStore.setState({ clip: null, request: 0 });
    vi.restoreAllMocks();
  });

  it("opens a MIDI clip in the piano roll on double-click, with automation lanes under the tracks", async () => {
    // jsdom has no canvas: clip waveforms just skip drawing.
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
    const { container, mock } = await renderWithMock(<App />);
    expect(container.querySelectorAll('[data-slot="automation"] [data-automation-track]').length).toBeGreaterThan(0);
    const chords = Object.values(useProjectStore.getState().project!.clips).find((c) => c.name === "Chords")!;
    await act(async () => {
      fireEvent.doubleClick(container.querySelector(`[data-clip-id="${chords.id}"]`)!);
    });
    expect(screen.getByRole("tab", { name: "Piano Roll" })).toHaveAttribute("aria-selected", "true");
    expect(container.querySelector('[data-slot="detail"] [data-testid="piano-roll"]')).not.toBeNull();
    mock.dispose();
  });
});

describe("App shell", () => {
  it("mounts every feature slot", () => {
    const { container } = render(<App />);
    for (const f of ["transport-bar", "project", "recording", "browser", "arrangement", "devices"]) {
      expect(container.querySelector(`[data-feature="${f}"]`), f).not.toBeNull();
    }
  });

  it("mounts the arrangement as the main view", () => {
    const { container } = render(<App />);
    expect(container.querySelector('[data-feature="arrangement"]')).not.toBeNull();
  });

  it("switches detail tabs", () => {
    const { container } = render(<App />);
    fireEvent.click(screen.getByRole("tab", { name: "Mixer" }));
    expect(container.querySelector('[data-feature="mixer"]')).not.toBeNull();
    fireEvent.click(screen.getByRole("tab", { name: "Plugins" }));
    expect(container.querySelector('[data-feature="plugins"]')).not.toBeNull();
  });
});
