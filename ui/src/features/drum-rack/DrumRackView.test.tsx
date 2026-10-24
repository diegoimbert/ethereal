import { act, fireEvent, screen, within } from "@testing-library/react";
import { pickOption } from "@/kit/testing";
import { pickDevice } from "@/features/devices/testing";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import type { Device, DrumPad } from "@/generated";
import { BROWSER_DRAG_MIME, type BrowserDragPayload } from "@/features/browser/dragPayload";
import { flush, renderWithMock, resetStores, store, stubPointerCapture, trackByName } from "@/features/devices/testUtils";
import { devicesOfPad, devicesOfTrack, useSelectionStore } from "@/state";
import { cmd } from "@/transport";
import { type MockTransport } from "@/transport/mock";
import { DrumRackView } from "./index";
import { bankOf, bankStarts, gridNotes, noteName, playableSlices } from "./padUtils";
import { useDrumFocus } from "./focusStore";
import { useDrumSolo } from "./soloStore";
import { stopPreview } from "@/features/piano-roll/notePreview";

let mock: MockTransport | undefined;
beforeAll(stubPointerCapture);
afterEach(() => {
  stopPreview();
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

  it("counts playable slices", () => {
    expect(playableSlices({ enabled: true, base_note: 126, markers: [0, 1, 2] })).toBe(2);
  });
});

describe("DrumRackView", () => {
  it("a pad focused from elsewhere is selected on its bank, again after the user moves away", async () => {
    await renderOnKeys();
    fireEvent.click(screen.getByRole("button", { name: "+ drum rack" }));
    await flush();
    const rack = rackOf("Keys")!;
    expect(screen.getByRole("region", { name: "empty pad C1" })).toBeDefined();

    act(() => useDrumFocus.getState().focusPad(rack.id, 60));
    expect(screen.getByRole("region", { name: "empty pad C3" })).toBeDefined();

    // The user picks another pad; the same note focused again reapplies.
    fireEvent.click(cell(61));
    expect(screen.getByRole("region", { name: "empty pad C#3" })).toBeDefined();
    act(() => useDrumFocus.getState().focusPad(rack.id, 60));
    expect(screen.getByRole("region", { name: "empty pad C3" })).toBeDefined();
    useDrumFocus.setState({ focus: null });
  });

  it("builds a 2-pad kit: rack, sample drop, pad settings, pad chain, solo", async () => {
    await renderOnKeys();
    expect(screen.getByTestId("drum-rack-view").textContent).toMatch(/no drum rack/);
    fireEvent.click(screen.getByRole("button", { name: "+ drum rack" }));
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
    expect(chain.map((d) => d.name)).toEqual(["sampler"]);
    expect(
      within(screen.getByTestId("pad-settings")).getByRole("region", {
        name: "sampler",
      }),
    ).toBeDefined();

    // Rename in place: double-click the pad's name, type, Enter.
    fireEvent.doubleClick(within(cell(36)).getByText(kick.name));
    const field = screen.getByRole("textbox", { name: "pad name" });
    fireEvent.change(field, { target: { value: "Big Kick" } });
    fireEvent.keyDown(field, { key: "Enter" });
    await flush();
    expect(store().project!.drum_pads[kick.id]!.name).toBe("Big Kick");
    expect(screen.queryByRole("textbox", { name: "pad name" })).toBeNull();

    // A second pad on D1 (another drop), then its settings.
    await drop(cell(38), "Drums/Snare.wav");
    expect(pads().map((p) => p.note)).toEqual([36, 38]);
    const snare = pads()[1]!;
    const settings = screen.getByTestId("pad-settings");
    expect(settings.getAttribute("aria-label")).toBe(`pad ${snare.name}`);
    pickOption(within(settings).getByRole("combobox", { name: "choke group" }), { value: "1" });
    await flush();
    expect(store().project!.drum_pads[snare.id]!.choke_group).toBe(1);
    // Mute / solo are on the pad itself (the track header's toggles).
    fireEvent.click(screen.getByRole("button", { name: `mute ${snare.name}` }));
    await flush();
    expect(store().project!.drum_pads[snare.id]!.mute).toBe(true);

    // Solo is runtime: no document change.
    const before = store().project;
    fireEvent.click(screen.getByRole("button", { name: `solo ${snare.name}` }));
    await flush();
    expect(useDrumSolo.getState().soloed.has(snare.id)).toBe(true);
    expect(store().project).toBe(before);

    // Add an effect to the pad chain.
    await pickDevice(within(settings).getAllByRole("button", { name: "add audio effect" })[0]!, "delay");
    await flush();
    expect(devicesOfPad(store().project!, snare.id).map((d) => d.name)).toEqual(["sampler", "delay"]);
    // Pad devices stay off the track chain.
    expect(devicesOfTrack(store().project!, trackByName("Keys").id).some((d) => d.pad !== null)).toBe(false);

    // Selecting the first pad shows its chain; clicking also auditions it.
    fireEvent.click(cell(36));
    await flush();
    expect(screen.getByTestId("pad-settings").getAttribute("aria-label")).toBe("pad Big Kick");

    // Undo the D1 drop step by step: mute, choke, device, then the whole drop at once.
    for (let i = 0; i < 3; i++) await act(() => mock!.send(cmd("Edit", { type: "Undo" })));
    expect(pads()).toHaveLength(2);
    await act(() => mock!.send(cmd("Edit", { type: "Undo" })));
    expect(pads().map((p) => p.note)).toEqual([36]);
  });

  it("pad audition holds the pad's note on the rack's track (through its chain), not a file preview", async () => {
    await renderOnKeys();
    fireEvent.click(screen.getByRole("button", { name: "+ drum rack" }));
    await flush();
    await drop(cell(36), "Drums/Kick.wav");
    const keys = trackByName("Keys").id;
    const sent: string[] = [];
    const send = mock!.send.bind(mock);
    mock!.send = (c, o) => {
      sent.push(`${c.domain}:${c.command.type}`);
      return send(c, o);
    };

    // Press: note on (selects the pad too); release anywhere: note off.
    fireEvent.pointerDown(cell(36), { button: 0 });
    fireEvent.click(cell(36));
    await flush();
    expect(mock!.previewNotes).toEqual([{ track: keys, data: [0x90, 36, 100] }]);
    expect(screen.getByTestId("pad-settings").getAttribute("aria-label")).toBe("pad Kick");
    fireEvent.pointerUp(window);
    await flush();
    expect(mock!.previewNotes.map((p) => p.data)).toEqual([
      [0x90, 36, 100],
      [0x80, 36, 0],
    ]);

    // No separate play button (pressing the pad plays it); the keyboard (Space) does the
    // same; a right click doesn't.
    expect(screen.queryByRole("button", { name: /preview/ })).toBeNull();
    fireEvent.pointerDown(cell(36), { button: 0 });
    fireEvent.pointerUp(window);
    fireEvent.keyDown(cell(36), { key: " " });
    fireEvent.keyUp(cell(36), { key: " " });
    fireEvent.pointerDown(cell(36), { button: 2 });
    // Empty notes have nothing to play.
    fireEvent.pointerDown(cell(37), { button: 0 });
    fireEvent.pointerUp(window);
    await flush();
    expect(mock!.previewNotes.map((p) => p.data)).toEqual([
      [0x90, 36, 100],
      [0x80, 36, 0],
      [0x90, 36, 100],
      [0x80, 36, 0],
      [0x90, 36, 100],
      [0x80, 36, 0],
    ]);
    expect(sent).not.toContain("Media:Preview");
  });

  it("exclusive pad solo: plain click solos only that pad of the rack, Cmd/Ctrl adds", async () => {
    await renderOnKeys();
    fireEvent.click(screen.getByRole("button", { name: "+ drum rack" }));
    await flush();
    await drop(cell(36), "Drums/Kick.wav");
    await drop(cell(38), "Drums/Snare.wav");
    await drop(cell(39), "Drums/Hat Closed.wav");
    const [kick, snare, clap] = pads() as [DrumPad, DrumPad, DrumPad];
    const soloed = () => [...useDrumSolo.getState().soloed].sort();
    const click = async (pad: DrumPad, mods: { metaKey?: boolean; ctrlKey?: boolean } = {}) => {
      fireEvent.click(screen.getByRole("button", { name: `solo ${pad.name}` }), mods);
      await flush();
    };
    await click(kick, { metaKey: true });
    await click(snare, { ctrlKey: true });
    await click(clap, { metaKey: true });
    expect(soloed()).toEqual([kick.id, snare.id, clap.id].sort());
    await click(snare);
    expect(soloed()).toEqual([snare.id]);
    await click(clap);
    expect(soloed()).toEqual([clap.id]);
    await click(clap);
    expect(soloed()).toEqual([]);
  });

  it("deletes a pad from its context menu", async () => {
    await renderOnKeys();
    fireEvent.click(screen.getByRole("button", { name: "+ drum rack" }));
    await flush();
    await drop(cell(36), "Drums/Kick.wav");
    fireEvent.contextMenu(cell(36));
    const { useContextMenuStore } = await import("@/kit");
    const item = useContextMenuStore.getState().menu!.items.find((i) => i !== "separator" && i.label === "delete pad");
    expect(item).toBeDefined();
    await act(async () => {
      if (item && item !== "separator") item.onSelect();
    });
    await flush();
    expect(pads()).toHaveLength(0);
    expect(Object.values(store().project!.devices).some((d) => d.pad !== null)).toBe(false);

    // Removing a pad's last device deletes the pad too.
    await drop(cell(38), "Drums/Snare.wav");
    fireEvent.click(cell(38));
    await flush();
    fireEvent.click(within(screen.getByTestId("pad-settings")).getByRole("button", { name: "remove sampler" }));
    await flush();
    expect(pads()).toHaveLength(0);
  });
});
