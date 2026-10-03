import { act, fireEvent, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { renderWithMock, resetStores } from "@/features/transport-bar/testUtils";
import { devicesOfTrack, tracksOrdered, useProjectStore, useSelectionStore } from "@/state";
import { DevicesPanel } from "./DevicesPanel";

afterEach(() => {
  resetStores();
  useSelectionStore.getState().selectTrack(null);
});

const project = () => useProjectStore.getState().project!;

describe("DevicesPanel", () => {
  it("adds a device to the selected track; instruments only on MIDI tracks", async () => {
    await renderWithMock(<DevicesPanel />);
    const drums = tracksOrdered(project()).find((t) => t.kind === "Audio")!;
    act(() => useSelectionStore.getState().selectTrack(drums.id));
    expect(await screen.findByText(drums.name)).toBeInTheDocument();
    expect(await screen.findByRole("button", { name: "Add Synth" })).toBeDisabled();
    const before = devicesOfTrack(project(), drums.id).length;
    fireEvent.click(screen.getByRole("button", { name: "Add Delay" }));
    await waitFor(() => expect(devicesOfTrack(project(), drums.id)).toHaveLength(before + 1));
    expect((await screen.findByRole("status")).textContent).toBe(`Added Delay to ${drums.name}`);

    const keys = tracksOrdered(project()).find((t) => t.kind === "Midi")!;
    act(() => useSelectionStore.getState().selectTrack(keys.id));
    expect(await screen.findByRole("button", { name: "Add Synth" })).toBeEnabled();
  });

  it("an added instrument replaces the track's instrument in place", async () => {
    await renderWithMock(<DevicesPanel />);
    const keys = tracksOrdered(project()).find((t) => t.kind === "Midi")!;
    act(() => useSelectionStore.getState().selectTrack(keys.id));
    const names = () =>
      devicesOfTrack(project(), keys.id)
        .filter((d) => !d.chain)
        .map((d) => (d.kind.type === "Builtin" ? d.kind.device.type : d.kind.type));
    const [instrument, ...rest] = names();
    expect(instrument).toBe("Synth");
    fireEvent.click(await screen.findByRole("button", { name: "Add Sampler" }));
    await waitFor(() => expect(names()).toEqual(["Sampler", ...rest]));
  });
});
