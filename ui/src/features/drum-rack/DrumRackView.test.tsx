import { act, fireEvent, screen, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import type { Device, DrumPad } from "@/generated";
import { BROWSER_DRAG_MIME, type BrowserDragPayload } from "@/features/browser/dragPayload";
import { flush, renderWithMock, resetStores, store, stubPointerCapture, trackByName } from "@/features/mixer/testUtils";
import { devicesOfPad, devicesOfTrack, useSelectionStore } from "@/state";
import { cmd, newId, type MockTransport } from "@/transport";
import { DrumRackView } from "./index";
import { bankOf, bankStarts, gridNotes, nearestMarker, noteName, playableSlices } from "./padUtils";
import { useDrumSolo } from "./soloStore";

let mock: MockTransport | undefined;
beforeAll(stubPointerCapture);
afterEach(() => {
  resetStores(mock);
  useDrumSolo.getState().reset();
  mock = undefined;
});

function payload(path: string) {
  const data = new Map<string, string>();
  const p: BrowserDragPayload = {
    version: 1,
    kind: "media",
    source: {
      type: "Location",
      location: { type: "Library", id: "library" },
      path,
    },
    name: path.split("/").pop()!,
    file_kind: "Audio",
  };
  data.set(BROWSER_DRAG_MIME, JSON.stringify(p));
  return {
    types: [...data.keys()],
    getData: (f: string) => data.get(f) ?? "",
    dropEffect: "none",
  };
}

const pads = (): DrumPad[] => Object.values(store().project!.drum_pads).sort((a, b) => a.note - b.note);
const rackOf = (track: string): Device | undefined =>
  devicesOfTrack(store().project!, trackByName(track).id).find((d) => d.kind.type === "Builtin" && d.kind.device.type === "DrumRack");
const cell = (note: number) => screen.getByRole("gridcell", { name: new RegExp(`^${noteName(note)} `) });

async function drop(el: Element, path: string) {
  const dt = payload(path);
  await act(async () => {
    fireEvent.dragOver(el, { dataTransfer: dt });
    fireEvent.drop(el, { dataTransfer: dt });
  });
  await flush();
  await flush();
}

async function renderOnKeys() {
  mock = await renderWithMock(<DrumRackView />);
  useSelectionStore.getState().selectTrack(trackByName("Keys").id);
  await flush();
}

describe("pad helpers", () => {
  it("names notes (C3 = 60) and lays banks out bottom-left up", () => {
    expect([noteName(36), noteName(60), noteName(0), noteName(127)]).toEqual(["C1", "C3", "C-2", "G8"]);
    expect(bankStarts()[0]).toBe(4);
    expect(bankOf(36)).toBe(36);
    expect(bankOf(51)).toBe(36);
    expect(bankOf(2)).toBe(4);
    const g = gridNotes(36);
    expect(g.slice(12)).toEqual([36, 37, 38, 39]);
    expect(g.slice(0, 4)).toEqual([48, 49, 50, 51]);
    expect(gridNotes(116).filter((n) => n === null)).toHaveLength(4);
  });

  it("counts playable slices and finds markers", () => {
    expect(playableSlices({ enabled: true, base_note: 126, markers: [0, 1, 2] })).toBe(2);
    expect(nearestMarker([0, 1, 2], 1.02, 0.05)).toBe(1);
    expect(nearestMarker([0, 1, 2], 1.5, 0.05)).toBe(-1);
  });
});

describe("DrumRackView", () => {
  it("builds a 2-pad kit: rack, sample drop, pad settings, pad chain, solo", async () => {
    await renderOnKeys();
    expect(screen.getByTestId("drum-rack-view").textContent).toMatch(/No drum rack/);
    fireEvent.click(screen.getByRole("button", { name: "+ Drum Rack" }));
    await flush();
    const rack = rackOf("Keys")!;
    expect(rack).toBeDefined();
    // Instruments go first in the chain.
    expect(devicesOfTrack(store().project!, trackByName("Keys").id)[0]!.id).toBe(rack.id);
    expect(screen.getAllByRole("gridcell")).toHaveLength(16);

    // Drop a sample on C1: pad + sampler in one undo step.
    await drop(cell(36), "Drums/Kick.wav");
    expect(pads().map((p) => p.note)).toEqual([36]);
    const kick = pads()[0]!;
    expect(kick.name).toMatch(/Kick/);
    const chain = devicesOfPad(store().project!, kick.id);
    expect(chain.map((d) => d.name)).toEqual(["Sampler"]);
    expect(
      within(screen.getByTestId("pad-settings")).getByRole("region", {
        name: "Sampler",
      }),
    ).toBeDefined();

    // A second pad on D1 (another drop), then its settings.
    await drop(cell(38), "Drums/Snare.wav");
    expect(pads().map((p) => p.note)).toEqual([36, 38]);
    const snare = pads()[1]!;
    const settings = screen.getByTestId("pad-settings");
    expect(settings.getAttribute("aria-label")).toBe(`Pad ${snare.name}`);
    fireEvent.change(within(settings).getByRole("combobox", { name: "Choke group" }), { target: { value: "1" } });
    await flush();
    expect(store().project!.drum_pads[snare.id]!.choke_group).toBe(1);
    fireEvent.click(within(settings).getByRole("button", { name: "Mute pad" }));
    await flush();
    expect(store().project!.drum_pads[snare.id]!.mute).toBe(true);

    // Solo is runtime: no document change.
    const before = store().project;
    fireEvent.click(within(settings).getByRole("button", { name: "Solo pad" }));
    await flush();
    expect(useDrumSolo.getState().soloed.has(snare.id)).toBe(true);
    expect(store().project).toBe(before);

    // Add an effect to the pad chain.
    fireEvent.change(within(settings).getByRole("combobox", { name: "Add device to pad" }), { target: { value: "Delay" } });
    await flush();
    expect(devicesOfPad(store().project!, snare.id).map((d) => d.name)).toEqual(["Sampler", "Delay"]);
    // Pad devices stay off the track chain.
    expect(devicesOfTrack(store().project!, trackByName("Keys").id).some((d) => d.pad !== null)).toBe(false);

    // Selecting the first pad shows its chain; clicking also auditions it.
    fireEvent.click(cell(36));
    await flush();
    expect(screen.getByTestId("pad-settings").getAttribute("aria-label")).toBe(`Pad ${kick.name}`);

    // Undo the D1 drop step by step: mute, choke, device, then the whole drop at once.
    for (let i = 0; i < 3; i++) await act(() => mock!.send(cmd("Edit", { type: "Undo" })));
    expect(pads()).toHaveLength(2);
    await act(() => mock!.send(cmd("Edit", { type: "Undo" })));
    expect(pads().map((p) => p.note)).toEqual([36]);
  });

  it("deletes a pad from its context menu", async () => {
    await renderOnKeys();
    fireEvent.click(screen.getByRole("button", { name: "+ Drum Rack" }));
    await flush();
    await drop(cell(36), "Drums/Kick.wav");
    fireEvent.contextMenu(cell(36));
    const { useContextMenuStore } = await import("@/kit");
    const item = useContextMenuStore.getState().menu!.items.find((i) => i !== "separator" && i.label === "Delete Pad");
    expect(item).toBeDefined();
    await act(async () => {
      if (item && item !== "separator") item.onSelect();
    });
    await flush();
    expect(pads()).toHaveLength(0);
    expect(Object.values(store().project!.devices).some((d) => d.pad !== null)).toBe(false);
  });

  it("slices a sampler (auto-slice, add/remove markers) and turns it into a drum rack", async () => {
    await renderOnKeys();
    const keys = trackByName("Keys");
    const media = Object.values(store().project!.media)[0]!;
    const sampler = newId();
    await act(() =>
      mock!.send(
        cmd("Device", {
          type: "Insert",
          id: sampler,
          track: keys.id,
          device: {
            type: "Builtin",
            device: {
              type: "Sampler",
              sample: media.id,
              slices: { enabled: false, base_note: 36, markers: [] },
            },
          },
          before: null,
        }),
      ),
    );
    await flush();
    const editor = screen.getByTestId("slice-editor");
    fireEvent.change(within(editor).getByRole("combobox", { name: "Auto-slice mode" }), { target: { value: "Equal" } });
    const countField = within(editor).getByRole("spinbutton", {
      name: "Slice count",
    });
    fireEvent.change(countField, { target: { value: "4" } });
    fireEvent.keyDown(countField, { key: "Enter" });
    fireEvent.click(within(editor).getByRole("button", { name: "Auto-slice" }));
    await flush();
    expect(within(editor).getByTestId("slice-count").textContent).toBe("4 slices");
    expect(within(editor).getAllByRole("slider", { name: /^Slice \d/ })).toHaveLength(4);
    // Remove one marker (double-click), put it back (click on the lane).
    fireEvent.doubleClick(within(editor).getByRole("slider", { name: "Slice 4" }));
    await flush();
    expect(within(screen.getByTestId("slice-editor")).getByTestId("slice-count").textContent).toBe("3 slices");
    await act(() => mock!.send(cmd("Edit", { type: "Undo" })));
    await flush();

    fireEvent.click(
      within(screen.getByTestId("slice-editor")).getByRole("button", {
        name: "Slice to Drum Rack",
      }),
    );
    await flush();
    const p = store().project!;
    expect(p.devices[sampler]).toBeUndefined();
    const rack = rackOf("Keys")!;
    const made = Object.values(p.drum_pads).filter((x) => x.rack === rack.id);
    expect(made.map((x) => x.name).sort()).toEqual(["Slice 1", "Slice 2", "Slice 3", "Slice 4"]);
    // The new rack shows its pads.
    expect(screen.getByRole("gridcell", { name: "C1 Slice 1" })).toBeDefined();
  });
});
