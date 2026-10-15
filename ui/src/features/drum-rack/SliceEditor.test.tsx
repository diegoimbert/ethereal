import { act, fireEvent, screen, within } from "@testing-library/react";
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type { Clip, Device, DrumPad, LibraryItem } from "@/generated";
import { BROWSER_DRAG_MIME, type BrowserDragPayload } from "@/features/browser/dragPayload";
import { flush, renderWithMock, resetStores, store, stubPointerCapture, trackByName } from "@/features/devices/testUtils";
import { DeviceEditor } from "@/features/devices/window/DeviceWindow";
import { useDeviceWindows } from "@/features/devices/window/store";
import { stopPreview } from "@/features/piano-roll/notePreview";
import { pickOption } from "@/kit/testing";
import { devicesOfPad, devicesOfTrack, tracksOrdered, useProjectStore, useSelectionStore } from "@/state";
import { cmd, CommandFailedError, newId, type EngineTransport, type MockTransport } from "@/transport";
import { DrumRackView } from "./index";
import { sliceClipEntries, sliceSampleEntries } from "./menus";
import { sendWithPeaks } from "./sliceActions";

let mock: MockTransport | undefined;
// jsdom has no layout: every element is 200 px wide (the lane maps pointer x by it).
const rect = { left: 0, top: 0, right: 200, bottom: 80, width: 200, height: 80, x: 0, y: 0, toJSON: () => ({}) };
beforeAll(() => {
  stubPointerCapture();
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockReturnValue(rect as DOMRect);
});
afterAll(() => vi.restoreAllMocks());
afterEach(() => {
  stopPreview();
  resetStores(mock);
  useDeviceWindows.setState({ open: [] });
  mock = undefined;
});

const undo = () => act(() => mock!.send(cmd("Edit", { type: "Undo" })).then(() => undefined));
const sampler = (id: string): Device | undefined => store().project!.devices[id];
const slices = (id: string) => {
  const k = sampler(id)!.kind;
  if (k.type !== "Builtin" || k.device.type !== "Sampler") throw new Error("not a sampler");
  return k.device.slices;
};
const padsOf = (rack: string): DrumPad[] =>
  Object.values(store().project!.drum_pads)
    .filter((p) => p.rack === rack)
    .sort((a, b) => a.note - b.note);
const racksOf = (track: string) => devicesOfTrack(store().project!, track).filter((d) => d.kind.type === "Builtin" && d.kind.device.type === "DrumRack");

/** The Sampler's window body, for the device `id`. */
function SamplerWindow({ id }: { id: string }) {
  const device = useProjectStore((s) => s.project?.devices[id]);
  return device ? <DeviceEditor device={device} /> : null;
}

/** A Sampler playing the demo's drum loop on Keys, in its window. */
async function samplerWindow(): Promise<string> {
  const id = newId();
  mock = await renderWithMock(<SamplerWindow id={id} />);
  const media = Object.values(store().project!.media)[0]!;
  await act(() =>
    mock!
      .send(
        cmd("Device", {
          type: "Insert",
          id,
          track: trackByName("Keys").id,
          device: { type: "Builtin", device: { type: "Sampler", sample: media.id, slices: { enabled: false, base_note: 36, markers: [] } } },
          before: null,
        }),
      )
      .then(() => undefined),
  );
  await flush();
  await flush();
  return id;
}

function payload(path: string): BrowserDragPayload {
  return {
    version: 1,
    kind: "media",
    source: { type: "Location", location: { type: "Library", id: "library" }, path },
    name: path.split("/").pop()!,
    file_kind: "Audio",
  };
}

function dataTransfer(p: BrowserDragPayload) {
  const data = new Map([[BROWSER_DRAG_MIME, JSON.stringify(p)]]);
  return { types: [...data.keys()], getData: (f: string) => data.get(f) ?? "", dropEffect: "none" };
}

describe("the Sampler's slice mode", () => {
  it("switches to slice mode with its transients in one step, and back", async () => {
    const id = await samplerWindow();
    expect(screen.queryByTestId("slice-editor")).toBeNull();
    expect(screen.getByRole("tab", { name: "classic" }).getAttribute("aria-selected")).toBe("true");

    fireEvent.click(screen.getByRole("tab", { name: "slice" }));
    await flush();
    // The mock finds 8 "transients" (equal parts).
    expect(slices(id).enabled).toBe(true);
    expect(slices(id).markers).toHaveLength(8);
    const editor = screen.getByTestId("slice-editor");
    expect(within(editor).getAllByRole("slider", { name: /^slice \d/ })).toHaveLength(8);
    expect(within(editor).getAllByTestId("slice-region")).toHaveLength(8);
    // The range knobs go with the range waveform.
    expect(screen.queryByTestId("widget-sample-waveform")).toBeNull();

    await undo();
    await flush();
    expect(slices(id)).toMatchObject({ enabled: false, markers: [] });
    expect(screen.queryByTestId("slice-editor")).toBeNull();

    // With markers already there, the switch only toggles the mode.
    await act(() => mock!.send(cmd("Slice", { type: "Add", device: id, positions: [0, 0.5] })).then(() => undefined));
    fireEvent.click(screen.getByRole("tab", { name: "slice" }));
    await flush();
    expect(slices(id)).toMatchObject({ enabled: true, markers: [0, 0.5] });
    fireEvent.click(screen.getByRole("tab", { name: "classic" }));
    await flush();
    expect(slices(id).enabled).toBe(false);
    expect(screen.getByTestId("widget-sample-waveform")).toBeDefined();
  });

  it("adds, moves and removes markers, each one undo step", async () => {
    const id = await samplerWindow();
    const media = Object.values(store().project!.media)[0]!;
    const length = media.frames / media.sample_rate;
    await act(() => mock!.send(cmd("Slice", { type: "Add", device: id, positions: [0] })).then(() => undefined));
    await act(() => mock!.send(cmd("Slice", { type: "SetEnabled", device: id, enabled: true })).then(() => undefined));
    await flush();
    const lane = () => screen.getByTestId("slice-lane");

    // Double-click at half the lane: a marker at half the sample.
    fireEvent.doubleClick(lane(), { clientX: 100 });
    await flush();
    expect(slices(id).markers).toHaveLength(2);
    expect(slices(id).markers[1]).toBeCloseTo(length / 2, 6);
    expect(screen.getByTestId("slice-count").textContent).toBe("2 slices");

    // Drag it to three quarters.
    const marker = screen.getByRole("slider", { name: "slice 2" });
    fireEvent.pointerDown(marker, { button: 0, pointerId: 1, clientX: 100 });
    fireEvent.pointerMove(lane(), { pointerId: 1, clientX: 150 });
    fireEvent.pointerUp(lane(), { pointerId: 1, clientX: 150 });
    await flush();
    expect(slices(id).markers[1]).toBeCloseTo((length * 3) / 4, 6);

    // Delete removes it; undo brings it back (and undoes the move separately).
    fireEvent.keyDown(screen.getByRole("slider", { name: "slice 2" }), { key: "Delete" });
    await flush();
    expect(slices(id).markers).toEqual([0]);
    await undo();
    expect(slices(id).markers[1]).toBeCloseTo((length * 3) / 4, 6);
    await undo();
    expect(slices(id).markers[1]).toBeCloseTo(length / 2, 6);
    await undo();
    expect(slices(id).markers).toEqual([0]);

    // A double-click on a marker removes it too.
    fireEvent.doubleClick(screen.getByRole("slider", { name: "slice 1" }));
    await flush();
    expect(slices(id).markers).toEqual([]);
  });

  it("auto-slices in equal parts, maps slices to keys from the first key, and plays a pressed slice", async () => {
    const id = await samplerWindow();
    const keys = trackByName("Keys").id;
    fireEvent.click(screen.getByRole("tab", { name: "slice" }));
    await flush();
    const editor = screen.getByTestId("slice-editor");
    pickOption(within(editor).getByRole("combobox", { name: "auto-slice mode" }), { value: "Equal" });
    const count = within(editor).getByRole("spinbutton", { name: "slice count" });
    fireEvent.change(count, { target: { value: "4" } });
    fireEvent.keyDown(count, { key: "Enter" });
    fireEvent.click(within(editor).getByRole("button", { name: "auto-slice" }));
    await flush();
    expect(slices(id).markers).toHaveLength(4);
    expect(screen.getByTestId("slice-count").textContent).toBe("4 slices");

    const first = screen.getByRole("spinbutton", { name: "first slice key" });
    fireEvent.change(first, { target: { value: "48" } });
    fireEvent.keyDown(first, { key: "Enter" });
    await flush();
    expect(slices(id).base_note).toBe(48);
    expect(within(screen.getByRole("slider", { name: "slice 2" })).getByText("C#2")).toBeDefined();

    // Press slice 2: its key (49) on the sampler's track, held until release.
    fireEvent.pointerDown(screen.getByRole("button", { name: "play slice 2" }), { button: 0 });
    await flush();
    expect(mock!.previewNotes).toEqual([{ track: keys, data: [0x90, 49, 100] }]);
    expect(screen.getByRole("button", { name: "play slice 2" }).getAttribute("aria-pressed")).toBe("true");
    fireEvent.pointerUp(window);
    await flush();
    expect(mock!.previewNotes.map((n) => n.data)).toEqual([
      [0x90, 49, 100],
      [0x80, 49, 0],
    ]);
  });

  it("slice to drum rack replaces the sampler by a rack with one pad per slice and opens its window", async () => {
    const id = await samplerWindow();
    const keys = trackByName("Keys").id;
    await act(() => mock!.send(cmd("Slice", { type: "Auto", device: id, mode: { type: "Equal", count: 3 } })).then(() => undefined));
    await flush();
    const before = racksOf(keys).length;
    fireEvent.click(screen.getByRole("button", { name: "slice to drum rack" }));
    await flush();
    expect(sampler(id)).toBeUndefined();
    const rack = racksOf(keys).at(-1)!;
    expect(racksOf(keys)).toHaveLength(before + 1);
    expect(padsOf(rack.id).map((p) => [p.note, p.name])).toEqual([
      [36, "Slice 1"],
      [37, "Slice 2"],
      [38, "Slice 3"],
    ]);
    expect(useDeviceWindows.getState().open).toContain(rack.id);
    await undo();
    expect(sampler(id)).toBeDefined();
    expect(racksOf(keys)).toHaveLength(before);
  });
});

describe("slice to pads", () => {
  it("a sample dropped on a drum rack's slice button fills the keys above its pads, in one undo step", async () => {
    mock = await renderWithMock(<DrumRackView />);
    useSelectionStore.getState().selectTrack(trackByName("Keys").id);
    await flush();
    fireEvent.click(screen.getByRole("button", { name: "+ drum rack" }));
    await flush();
    const rack = racksOf(trackByName("Keys").id)[0]!;
    await act(() => mock!.send(cmd("DrumRack", { type: "AddPad", id: newId(), rack: rack.id, note: 36, name: null })).then(() => undefined));
    await flush();
    const button = screen.getByTestId("slice-to-pads");
    const dt = dataTransfer(payload("Drums/Kick.wav"));
    await act(async () => {
      fireEvent.dragOver(button, { dataTransfer: dt });
      fireEvent.drop(button, { dataTransfer: dt });
    });
    for (let i = 0; i < 4; i++) await flush();
    const pads = padsOf(rack.id);
    // The mock's 8 "transients", from C#1 (37) up.
    expect(pads.map((p) => p.note)).toEqual([36, 37, 38, 39, 40, 41, 42, 43, 44]);
    expect(pads[1]!.name).toBe("Slice 1");
    const chain = devicesOfPad(store().project!, pads[1]!.id);
    expect(chain).toHaveLength(1);
    expect(chain[0]!.kind).toMatchObject({ type: "Builtin", device: { type: "Sampler", slices: { enabled: false } } });
    // The first slice's pad is selected.
    expect(screen.getByTestId("pad-settings").getAttribute("aria-label")).toBe("pad Slice 1");

    const media = Object.keys(store().project!.media).length;
    await undo();
    expect(padsOf(rack.id).map((p) => p.note)).toEqual([36]);
    expect(Object.keys(store().project!.media).length).toBeLessThan(media);
  });

  it("an audio clip's menu slices its sample to a new drum rack track below it", async () => {
    mock = await renderWithMock(<div />);
    const project = store().project!;
    const clip = Object.values(project.clips).find((c): c is Clip => c.content.type === "Audio")!;
    expect(sliceClipEntries(mock, Object.values(project.clips).find((c) => c.content.type === "Midi")!)).toEqual([]);
    const [entry] = sliceClipEntries(mock, clip);
    expect(entry).toMatchObject({ label: "slice to drum rack" });
    const tracks = tracksOrdered(project).map((t) => t.id);
    await act(async () => {
      if (entry && entry !== "separator") entry.onSelect();
    });
    for (let i = 0; i < 3; i++) await flush();
    const after = tracksOrdered(store().project!);
    expect(after).toHaveLength(tracks.length + 1);
    const made = after[after.findIndex((t) => t.id === clip.track) + 1]!;
    expect(made).toMatchObject({ kind: "Midi", name: clip.name });
    const [rack] = racksOf(made.id);
    expect(padsOf(rack!.id)).toHaveLength(8);
    await undo();
    expect(tracksOrdered(store().project!)).toHaveLength(tracks.length);
  });

  it("a browser sample's menu slices it to a new track, or onto the selected track's drum rack", async () => {
    mock = await renderWithMock(<div />);
    const item = {
      id: "x",
      kind: "Audio",
      name: "Kick.wav",
      root: "library",
      path: "Drums/Kick.wav",
      source: payload("Drums/Kick.wav").source,
      preset: null,
      tags: [],
      favourite: false,
      meta: {},
    } as unknown as LibraryItem;
    const keys = trackByName("Keys").id;
    useSelectionStore.getState().selectTrack(keys);
    expect(sliceSampleEntries(mock, item).map((e) => e !== "separator" && e.label)).toEqual(["slice to new drum rack"]);
    const count = tracksOrdered(store().project!).length;
    const [toNew] = sliceSampleEntries(mock, item);
    await act(async () => {
      if (toNew && toNew !== "separator") toNew.onSelect();
    });
    for (let i = 0; i < 4; i++) await flush();
    const made = tracksOrdered(store().project!).find((t) => t.name === "Kick")!;
    expect(tracksOrdered(store().project!)).toHaveLength(count + 1);
    expect(padsOf(racksOf(made.id)[0]!.id)).toHaveLength(8);

    // With a drum rack on the selected track: onto its pads.
    const rack = newId();
    await act(() =>
      mock!
        .send(cmd("Device", { type: "Insert", id: rack, track: keys, device: { type: "Builtin", device: { type: "DrumRack" } }, before: null }))
        .then(() => undefined),
    );
    const entries = sliceSampleEntries(mock, item);
    expect(entries.map((e) => e !== "separator" && e.label)).toEqual(["slice to new drum rack", "slice to the selected drum rack"]);
    await act(async () => {
      const e = entries[1];
      if (e && e !== "separator") e.onSelect();
    });
    for (let i = 0; i < 4; i++) await flush();
    expect(padsOf(rack).map((p) => p.note)).toEqual([36, 37, 38, 39, 40, 41, 42, 43]);
  });
});

describe("sendWithPeaks", () => {
  it("waits for a new sample's peaks, then sends again", async () => {
    let listener: ((e: unknown) => void) | null = null;
    let calls = 0;
    const transport = {
      onEvent: (l: (e: unknown) => void) => {
        listener = l;
        return () => {
          listener = null;
        };
      },
      send: () => {
        calls += 1;
        if (calls === 1)
          return Promise.reject(new CommandFailedError({ code: "InvalidState", message: "loading", key: { id: "error.drum_rack.sample_loading", args: {} } }));
        return Promise.resolve({ type: "Unit" });
      },
    } as unknown as EngineTransport;
    const done = sendWithPeaks(transport, cmd("Slice", { type: "SetEnabled", device: "d", enabled: true }), "m");
    await flush();
    expect(calls).toBe(1);
    listener!({ type: "Media", event: { type: "PeaksReady", media: "other" } });
    await flush();
    expect(calls).toBe(1);
    listener!({ type: "Media", event: { type: "PeaksReady", media: "m" } });
    await done;
    expect(calls).toBe(2);
    expect(listener).toBeNull();
  });

  it("other failures are not retried", async () => {
    const transport = {
      onEvent: () => () => {},
      send: () => Promise.reject(new CommandFailedError({ code: "InvalidArgument", message: "nope" })),
    } as unknown as EngineTransport;
    await expect(sendWithPeaks(transport, cmd("Slice", { type: "SetEnabled", device: "d", enabled: true }), "m")).rejects.toThrow(/nope/);
  });
});
