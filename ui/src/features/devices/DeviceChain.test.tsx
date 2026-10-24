import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { useDeviceClipboard } from "./deviceClipboard";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import type { Device } from "@/generated";
import { devicesOfTrack, useProjectStore, useSelectionStore } from "@/state";
import { cmd, newId } from "@/transport";
import { type MockTransport } from "@/transport/mock";
import { addTrackCommand } from "@/features/arrangement/actions";
import { dragUp, flush, renderWithMock, resetStores, store, stubPointerCapture, trackByName } from "@/features/devices/testUtils";
import { groupParams, insertableTypes, splitMainParams } from "./chainUtils";
import { BUILTIN_DESCRIPTORS } from "@/transport";
import { DeviceChain } from "./index";
import { collapseAll, resetCollapsed } from "./collapsed";
import { DeviceWindowsRoot, resetDeviceWindows } from "./window";
import { pickOption } from "@/kit/testing";
import { useContextMenuStore, type ContextMenuItem } from "@/kit/contextMenuStore";
import { pickDevice } from "@/features/devices/testing";
import { BROWSER_DRAG_MIME, type BrowserDragPayload } from "@/features/browser/dragPayload";

let mock: MockTransport | undefined;
beforeAll(stubPointerCapture);
afterEach(() => {
  resetStores(mock);
  mock = undefined;
  resetDeviceWindows();
  resetCollapsed();
});

const chainNames = (track: string) => devicesOfTrack(store().project!, trackByName(track).id).map((d) => d.name);
const deviceOf = (track: string, name: string): Device => devicesOfTrack(store().project!, trackByName(track).id).find((d) => d.name === name)!;
const deviceEl = (name: string) => screen.getByRole("region", { name });
/** A factory device's UI is in its window: opens it (header toggle), returns it. */
function openWindow(name: string): HTMLElement {
  const toggle = within(deviceEl(name)).getByRole("button", { name: `open ${name} window` });
  if (toggle.getAttribute("aria-pressed") !== "true") fireEvent.click(toggle);
  return screen.getByRole("dialog", { name: `${name} window` });
}

/** A fake DataTransfer carrying a sample-browser payload for a library file. */
function writePayload(path: string) {
  const data = new Map<string, string>();
  const payload: BrowserDragPayload = {
    version: 1,
    kind: "media",
    source: { type: "Location", location: { type: "Library", id: "library" }, path },
    name: path.split("/").pop()!,
    file_kind: "Audio",
  };
  data.set(BROWSER_DRAG_MIME, JSON.stringify(payload));
  return { types: [...data.keys()], getData: (f: string) => data.get(f) ?? "", dropEffect: "none" };
}

async function renderChain() {
  mock = await renderWithMock(
    <>
      <DeviceChain />
      <DeviceWindowsRoot />
    </>,
  );
  await screen.findByRole("region", { name: "Synth" });
}

describe("chain helpers", () => {
  it("only offers instruments on MIDI tracks", () => {
    // The v0.1/v2 built-ins (the v0.2 ones follow them in `BUILTIN_DESCRIPTORS`).
    const all = Object.values(BUILTIN_DESCRIPTORS).slice(0, 9);
    const midi = { kind: "Midi" } as Parameters<typeof insertableTypes>[1];
    const audio = { kind: "Audio" } as Parameters<typeof insertableTypes>[1];
    expect(insertableTypes(all, midi).map((d) => d.name)).toEqual([
      "Synth",
      "Sampler",
      "Compressor",
      "Delay",
      "EQ",
      "Reverb",
      "Limiter",
      "Utility",
      "Drum Rack",
    ]);
    expect(insertableTypes(all, audio).map((d) => d.name)).toEqual(["Compressor", "Delay", "EQ", "Reverb", "Limiter", "Utility"]);
  });

  it("keeps leading whole groups as the main controls and folds the rest", () => {
    // The generic layout (devices without a declared one, e.g. plugins).
    const { main, more } = splitMainParams(groupParams(BUILTIN_DESCRIPTORS.Synth.params));
    expect(main.map((g) => g.group)).toEqual(["Oscillator"]);
    expect(more.map((g) => g.group)).toEqual(["Envelope", "Filter", "Output", "Voice"]);
    const small = groupParams(BUILTIN_DESCRIPTORS.Limiter.params);
    expect(splitMainParams(small).more).toEqual([]);
  });

  it("groups visible params by section", () => {
    const groups = groupParams(BUILTIN_DESCRIPTORS.Synth.params);
    expect(groups.map((g) => g.group)).toEqual(["Oscillator", "Envelope", "Filter", "Output", "Voice"]);
    expect(groupParams([{ ...BUILTIN_DESCRIPTORS.Delay.params[0]!, hidden: true }])).toEqual([]);
  });
});

describe("DeviceChain", () => {
  it("shows the selected track's chain (first track by default) with params from the descriptor", async () => {
    await renderChain();
    expect(screen.getByRole("combobox", { name: "track" })).toHaveTextContent("Keys");
    const list = screen.getByRole("list", { name: "Keys devices" });
    expect(
      within(list)
        .getAllByRole("region")
        .map((r) => r.getAttribute("aria-label")),
    ).toEqual(["Synth", "Compressor"]);
    // The card is the header only: the device's own UI is in its window.
    expect(within(deviceEl("Synth")).queryByRole("slider")).toBeNull();
    const synth = openWindow("Synth");
    await within(synth).findByRole("slider", { name: "cutoff" });
    // The Synth's declared layout: the oscillator's shape choice (4 labels → select) and the
    // filter curve's cutoff knob showing the formatted plain value.
    expect(within(synth).getByRole("combobox", { name: "waveform" })).toHaveTextContent("saw");
    expect(within(synth).getByRole("slider", { name: "cutoff" })).toHaveAttribute("aria-valuetext", "2.4 khz");

    act(() => useSelectionStore.getState().selectTrack(trackByName("Drums").id));
    expect(screen.getByRole("list", { name: "Drums devices" })).toBeInTheDocument();
    expect(screen.getByRole("combobox", { name: "track" })).toHaveTextContent("Drums");
  });

  it("renames a device instance; an empty name brings back the device's own", async () => {
    await renderChain();
    fireEvent.doubleClick(within(deviceEl("Synth")).getByText("Synth"));
    const field = screen.getByRole("textbox", { name: "rename Synth" });
    fireEvent.change(field, { target: { value: "Lead" } });
    fireEvent.keyDown(field, { key: "Enter" });
    await flush();
    expect(chainNames("Keys")[0]).toBe("Lead");

    fireEvent.doubleClick(within(deviceEl("Lead")).getByText("Lead"));
    const again = screen.getByRole("textbox", { name: "rename Lead" });
    fireEvent.change(again, { target: { value: "  " } });
    fireEvent.keyDown(again, { key: "Enter" });
    await flush();
    expect(chainNames("Keys")[0]).toBe("synth");
  });

  it("toggles bypass", async () => {
    await renderChain();
    fireEvent.click(screen.getByRole("button", { name: "bypass Compressor" }));
    await flush();
    expect(deviceOf("Keys", "Compressor").enabled).toBe(false);
    expect(deviceEl("Compressor")).toHaveClass("eth-device--bypassed");
    fireEvent.click(screen.getByRole("button", { name: "enable Compressor" }));
    await flush();
    expect(deviceOf("Keys", "Compressor").enabled).toBe(true);
  });

  it("adds, reorders and removes devices", async () => {
    await renderChain();
    await pickDevice(screen.getByRole("button", { name: "add device" }), "delay");
    await flush();
    expect(chainNames("Keys")).toEqual(["Synth", "Compressor", "delay"]);

    fireEvent.click(screen.getByRole("button", { name: "move delay left" }));
    await flush();
    expect(chainNames("Keys")).toEqual(["Synth", "delay", "Compressor"]);

    // Moves stay inside a section: the instrument is alone in its own.
    expect(screen.getByRole("button", { name: "move Synth right" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "move delay left" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "move delay right" }));
    await flush();
    expect(chainNames("Keys")).toEqual(["Synth", "Compressor", "delay"]);
    expect(screen.getByRole("button", { name: "move delay right" })).toBeDisabled();
    expect(
      within(screen.getByRole("group", { name: "audio effects" }))
        .getAllByRole("region")
        .map((r) => r.getAttribute("aria-label")),
    ).toEqual(["Compressor", "delay"]);

    fireEvent.click(screen.getByRole("button", { name: "remove delay" }));
    await flush();
    expect(chainNames("Keys")).toEqual(["Synth", "Compressor"]);
  });

  it("an added instrument replaces the track's instrument in place", async () => {
    await renderChain();
    await pickDevice(screen.getByRole("button", { name: "add device" }), "sampler");
    await flush();
    expect(chainNames("Keys")).toEqual(["sampler", "Compressor"]);
  });

  it("replaces the default Synth of a new MIDI track (one undo step)", async () => {
    await renderChain();
    const id = newId();
    await act(() => mock!.send(addTrackCommand("Midi", id, newId())));
    act(() => useSelectionStore.getState().selectTrack(id));
    await flush();
    const name = store().project!.tracks[id]!.name;
    expect(chainNames(name)).toEqual(["synth"]);
    await pickDevice(screen.getByRole("button", { name: "add device" }), "sampler");
    await flush();
    expect(chainNames(name)).toEqual(["sampler"]);
    await act(() => mock!.send(cmd("Edit", { type: "Undo" })));
    expect(chainNames(name)).toEqual(["synth"]);
  });

  it("inserts an instrument at the start of a chain without one", async () => {
    await renderChain();
    const id = newId();
    await act(() => mock!.send(cmd("Track", { type: "Create", id, kind: "Midi", name: "Bare", color: null, parent: null, before: null })));
    act(() => useSelectionStore.getState().selectTrack(id));
    await flush();
    await pickDevice(screen.getByRole("button", { name: "add device" }), "delay");
    await flush();
    await pickDevice(screen.getByRole("button", { name: "add device" }), "sampler");
    await flush();
    expect(chainNames("Bare")).toEqual(["sampler", "delay"]);
  });

  it("loads a sample dropped from the browser into a Sampler (one undo step)", async () => {
    await renderChain();
    await pickDevice(screen.getByRole("button", { name: "add device" }), "sampler");
    await flush();
    const win = openWindow("sampler");
    // Once its descriptor is in (the layout's waveform widget carries the slot).
    await waitFor(() => expect(within(win).queryByText("loading…")).toBeNull());
    const slot = within(win).getByTestId("sample-slot");
    expect(slot.textContent).toMatch(/drop or select a sample/i);
    const payload = writePayload("Drums/Kick.wav");
    await act(async () => {
      fireEvent.dragOver(slot, { dataTransfer: payload });
      fireEvent.drop(slot, { dataTransfer: payload });
    });
    await flush();
    const sampler = deviceOf("Keys", "sampler");
    const kind = sampler.kind.type === "Builtin" ? sampler.kind.device : null;
    const media = kind?.type === "Sampler" ? kind.sample : null;
    expect(media).not.toBeNull();
    expect(store().project!.media[media!]!.name).toMatch(/Kick/);
    expect(slot.textContent).toMatch(/Kick/);
    await act(async () => {
      await mock!.send(cmd("Edit", { type: "Undo" }));
    });
    const after = deviceOf("Keys", "sampler").kind;
    expect(after.type === "Builtin" && after.device.type === "Sampler" ? after.device.sample : "?").toBeNull();
  });

  it("a knob drag is one undo step and maps through the param scale", async () => {
    await renderChain();
    const before = deviceOf("Keys", "Synth").params[6] ?? 8000;
    const undoLabel = store().history.undo_label;
    await dragUp(await within(openWindow("Synth")).findByRole("slider", { name: "cutoff" }), 3, 10);
    await flush();
    const after = deviceOf("Keys", "Synth").params[6]!;
    // Log scale: +30px at 1/150 per px = +0.2 normalized = ×(1000^0.2) in Hz.
    expect(after / before).toBeCloseTo(Math.pow(1000, 0.2), 6);
    await act(() => mock!.send(cmd("Edit", { type: "Undo" })));
    expect(deviceOf("Keys", "Synth").params[6] ?? 8000).toBe(before);
    expect(store().history.undo_label).toBe(undoLabel);
  });

  it("sets enum params from the choice list and resets on double-click", async () => {
    await renderChain();
    pickOption(await within(openWindow("Synth")).findByRole("combobox", { name: "waveform" }), { value: "2" });
    await flush();
    expect(deviceOf("Keys", "Synth").params[0]).toBe(2);

    // The envelope widget's attack control (param 2 in `ether_devices::synth`).
    const attack = within(openWindow("Synth")).getByRole("slider", { name: "attack" });
    fireEvent.keyDown(attack, { key: "PageUp" });
    await flush();
    expect(deviceOf("Keys", "Synth").params[2]).toBeGreaterThan(5);
    fireEvent.doubleClick(attack);
    await flush();
    expect(deviceOf("Keys", "Synth").params[2]).toBeCloseTo(5, 9);
  });

  it("moves a dragged device before a drop target of its section only", async () => {
    await renderChain();
    const compressor = deviceOf("Keys", "Compressor");
    const data = new Map<string, string>([["application/x-ethereal-device", compressor.id]]);
    const dataTransfer = {
      types: [...data.keys()],
      getData: (k: string) => data.get(k) ?? "",
      setData: (k: string, v: string) => data.set(k, v),
    };
    fireEvent.dragOver(deviceEl("Synth"), { dataTransfer });
    fireEvent.drop(deviceEl("Synth"), { dataTransfer });
    await flush();
    // An audio effect never goes before the instrument.
    expect(chainNames("Keys")).toEqual(["Synth", "Compressor"]);
    await pickDevice(screen.getByRole("button", { name: "add device" }), "delay");
    await flush();
    const delay = deviceOf("Keys", "delay");
    data.set("application/x-ethereal-device", delay.id);
    // While dragging, a line shows where the device lands: only over targets it may take.
    fireEvent.dragStart(deviceEl("delay").querySelector("header")!, { dataTransfer });
    fireEvent.dragOver(deviceEl("Synth"), { dataTransfer });
    expect(screen.queryByTestId("device-insert")).toBeNull();
    fireEvent.dragOver(deviceEl("Compressor"), { dataTransfer });
    expect(screen.getByTestId("device-insert").nextElementSibling).toBe(deviceEl("Compressor"));
    fireEvent.drop(deviceEl("Compressor"), { dataTransfer });
    await flush();
    expect(chainNames("Keys")).toEqual(["Synth", "delay", "Compressor"]);
    expect(screen.queryByTestId("device-insert")).toBeNull();
    fireEvent.dragEnd(deviceEl("delay").querySelector("header")!);
  });
});

describe("dragging a multi-selection", () => {
  it("moves every selected card of the grabbed one's kind to the drop place, one undo step", async () => {
    await renderChain();
    const keys = trackByName("Keys").id;
    for (const type of ["Delay", "Reverb"] as const)
      await act(() => mock!.send(cmd("Device", { type: "Insert", id: newId(), track: keys, device: { type: "Builtin", device: { type } }, before: null })));
    await flush();
    const names = chainNames("Keys");
    expect(names.length).toBe(4);
    const [synth, compressor, delay, reverb] = names as [string, string, string, string];
    const header = (name: string) => deviceEl(name).querySelector("header")!;
    fireEvent.pointerDown(header(synth), { button: 0 });
    fireEvent.pointerDown(header(compressor), { button: 0, shiftKey: true });
    fireEvent.pointerDown(header(delay), { button: 0, metaKey: true });
    // A press on a selected card keeps the selection (the drag takes all of it).
    fireEvent.pointerDown(header(compressor), { button: 0 });
    expect(useDeviceClipboard.getState().selected.length).toBe(3);
    const data = new Map<string, string>();
    const dataTransfer = { types: [] as string[], getData: (k: string) => data.get(k) ?? "", setData: (k: string, v: string) => void data.set(k, v) };
    fireEvent.dragStart(header(compressor), { dataTransfer });
    dataTransfer.types = [...data.keys()];
    const end = document.querySelector(".eth-devices__end")!;
    fireEvent.dragOver(end, { dataTransfer });
    fireEvent.drop(end, { dataTransfer });
    fireEvent.dragEnd(header(compressor));
    await flush();
    // The instrument stays in its section; the two audio effects land at the end, in order.
    expect(chainNames("Keys")).toEqual([synth, reverb, compressor, delay]);
    await act(() => mock!.send(cmd("Edit", { type: "Undo" })));
    await flush();
    expect(chainNames("Keys")).toEqual(names);
  });
});

describe("DeviceChain: stacked layout (inspector)", () => {
  /** The Keys track's chain, stacked, once the project has loaded. */
  function KeysStack() {
    const id = useProjectStore((s) => Object.values(s.project?.tracks ?? {}).find((t) => t.name === "Keys")?.id);
    return id ? <DeviceChain track={id} layout="stack" /> : null;
  }

  afterEach(() => resetCollapsed());

  async function renderStack() {
    mock = await renderWithMock(
      <>
        <KeysStack />
        <DeviceWindowsRoot />
      </>,
    );
    await screen.findByRole("region", { name: "Synth" });
  }

  it("shows the given track's chain as full-width cards, no track picker, an add menu per section", async () => {
    await renderStack();
    expect(screen.queryByRole("combobox", { name: "track" })).toBeNull();
    const list = screen.getByRole("list", { name: "Keys devices" });
    expect(list).toHaveClass("eth-devices__chain--stack");
    expect(
      within(list)
        .getAllByRole("region")
        .map((r) => r.getAttribute("aria-label")),
    ).toEqual(["Synth", "Compressor"]);
    // Each section adds its own kind of device, opening the device library of that kind: a
    // "+" right of its label and a placeholder card last in it; none for the instrument while
    // there is one.
    expect(screen.queryByRole("button", { name: "add device" })).toBeNull();
    const midiFx = within(screen.getByRole("group", { name: "midi effects" }));
    expect(midiFx.getAllByRole("button", { name: "add midi effect" })).toHaveLength(2);
    expect(screen.queryByRole("button", { name: "add instrument" })).toBeNull();
    const fxCard = within(screen.getByRole("group", { name: "audio effects" }))
      .getAllByRole("button", { name: "add audio effect" })
      .at(-1)!;
    expect(fxCard).toHaveClass("eth-devices__add-card");
    expect(fxCard).toHaveTextContent("add audio effect");

    fireEvent.click(fxCard);
    const effects = await screen.findByRole("list", { name: "effects" });
    expect(document.activeElement).toBe(screen.getByRole("searchbox", { name: "search effects" }));
    fireEvent.click(await within(effects).findByRole("button", { name: "delay" }));
    await flush();
    expect(chainNames("Keys")).toEqual(["Synth", "Compressor", "delay"]);
    await waitFor(() => expect(screen.queryByRole("list", { name: "effects" })).toBeNull());
    // A device added from the library opens its window.
    expect(await screen.findByRole("dialog", { name: "delay window" })).toBeInTheDocument();

    fireEvent.click(midiFx.getAllByRole("button", { name: "add midi effect" })[0]!);
    const search = await screen.findByRole("searchbox", { name: "search midi effects" });
    fireEvent.change(search, { target: { value: "arp" } });
    fireEvent.keyDown(search, { key: "ArrowDown" });
    fireEvent.keyDown(search, { key: "Enter" });
    await flush();
    expect(chainNames("Keys")[0]).toBe("arpeggiator");
  });

  it("a drop at the chain's end shows its line between the last device and the add card, and lands there", async () => {
    await renderStack();
    await pickDevice(
      within(screen.getByRole("group", { name: "audio effects" }))
        .getAllByRole("button", { name: "add audio effect" })
        .at(-1)!,
      "delay",
    );
    await flush();
    expect(chainNames("Keys")).toEqual(["Synth", "Compressor", "delay"]);
    const compressor = deviceOf("Keys", "Compressor");
    const data = new Map<string, string>([["application/x-ethereal-device", compressor.id]]);
    const dataTransfer = { types: [...data.keys()], getData: (k: string) => data.get(k) ?? "", setData: (k: string, v: string) => data.set(k, v) };
    const header = deviceEl("Compressor").querySelector("header")!;
    const addCard = () =>
      within(screen.getByRole("group", { name: "audio effects" }))
        .getAllByRole("button", { name: "add audio effect" })
        .at(-1)!;
    const endZone = document.querySelector(".eth-devices__end")!;
    fireEvent.dragStart(header, { dataTransfer });
    // Over the end zone (below the add card): the line is in the add card's top gap, after the
    // last device in DOM order, never below the card.
    fireEvent.dragOver(endZone, { dataTransfer });
    let line = screen.getByTestId("device-insert");
    expect(line.parentElement).toBe(addCard());
    expect(deviceEl("delay").compareDocumentPosition(line) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(endZone).not.toHaveClass("eth-devices__end--drop");
    fireEvent.dragLeave(endZone);
    expect(screen.queryByTestId("device-insert")).toBeNull();
    // Over the add card itself: the same line; the drop moves the device to the end.
    fireEvent.dragOver(addCard(), { dataTransfer });
    line = screen.getByTestId("device-insert");
    expect(line.parentElement).toBe(addCard());
    fireEvent.drop(addCard(), { dataTransfer });
    await flush();
    expect(chainNames("Keys")).toEqual(["Synth", "delay", "Compressor"]);
    expect(screen.queryByTestId("device-insert")).toBeNull();
    fireEvent.dragEnd(header);
  });

  it("drags a device out of a section's library onto the chain: in its section only", async () => {
    await renderStack();
    const add = within(screen.getByRole("group", { name: "audio effects" })).getAllByRole("button", { name: "add audio effect" })[0]!;
    fireEvent.click(add);
    const popover = await screen.findByRole("dialog", { name: "add audio effect" });
    const row = await within(popover).findByRole("button", { name: "delay" });
    expect(row).toHaveAttribute("draggable", "true");
    const data = new Map<string, string>();
    const dataTransfer = {
      get types() {
        return [...data.keys()];
      },
      getData: (k: string) => data.get(k) ?? "",
      setData: (k: string, v: string) => void data.set(k, v),
      dropEffect: "copy",
    };
    fireEvent.dragStart(row, { dataTransfer });
    expect(data.get("application/x-ethereal-library-device")).toContain('"category":"AudioEffect"');
    // Not before the instrument (another section)...
    fireEvent.dragOver(deviceEl("Synth"), { dataTransfer });
    expect(screen.queryByTestId("device-insert")).toBeNull();
    // ...before an audio effect: the line, and the drop adds it there.
    fireEvent.dragOver(deviceEl("Compressor"), { dataTransfer });
    expect(screen.getByTestId("device-insert")).toBeInTheDocument();
    fireEvent.drop(deviceEl("Compressor"), { dataTransfer });
    fireEvent.dragEnd(row, { dataTransfer });
    await flush();
    expect(chainNames("Keys")).toEqual(["Synth", "delay", "Compressor"]);
    // Dropped: the library closed.
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "add audio effect" })).toBeNull());
  });

  it("selects several cards (⌘-click, Shift-click) and copies them all", async () => {
    await renderStack();
    const press = (name: string, mods: { metaKey?: boolean; shiftKey?: boolean } = {}) =>
      fireEvent.pointerDown(deviceEl(name).querySelector("header")!, { button: 0, ...mods });
    press("Synth");
    press("Compressor", { metaKey: true });
    expect(deviceEl("Synth")).toHaveClass("eth-device--selected");
    expect(deviceEl("Compressor")).toHaveClass("eth-device--selected");
    act(() => void document.dispatchEvent(new Event("copy")));
    expect(useDeviceClipboard.getState().copied).toEqual([deviceOf("Keys", "Synth").id, deviceOf("Keys", "Compressor").id]);
    // ⌘-click again: out of the selection; a plain click: only that one.
    press("Synth", { metaKey: true });
    expect(deviceEl("Synth")).not.toHaveClass("eth-device--selected");
    press("Synth");
    press("Compressor", { shiftKey: true });
    expect(useDeviceClipboard.getState().selected).toEqual([deviceOf("Keys", "Synth").id, deviceOf("Keys", "Compressor").id]);
  });

  it("⌘G / Ctrl+G groups the selected cards into a rack (an instrument among them: an instrument rack), one undo step", async () => {
    await renderStack();
    const press = (name: string, mods: { shiftKey?: boolean } = {}) => fireEvent.pointerDown(deviceEl(name).querySelector("header")!, { button: 0, ...mods });
    press("Synth");
    press("Compressor", { shiftKey: true });
    fireEvent.keyDown(deviceEl("Compressor"), { key: "g", code: "KeyG", ctrlKey: true });
    await flush();
    const top = devicesOfTrack(store().project!, trackByName("Keys").id);
    expect(top.map((d) => d.kind.type === "Builtin" && d.kind.device.type)).toEqual(["InstrumentRack"]);
    const rack = top[0]!;
    const inside = Object.values(store().project!.devices).filter((d) => d.chain != null && store().project!.rack_chains[d.chain]?.rack === rack.id);
    expect(inside.map((d) => d.name).sort()).toEqual(["Compressor", "Synth"]);
    expect(useDeviceClipboard.getState().selected).toEqual([rack.id]);
    await act(() => mock!.send(cmd("Edit", { type: "Undo" })));
    expect(chainNames("Keys")).toEqual(["Synth", "Compressor"]);
  });

  it("on/off and delete on a card of a multi-selection act on all of it; a plain click on the card then selects it alone", async () => {
    await renderStack();
    const press = (el: Element, mods: { shiftKey?: boolean } = {}) => {
      fireEvent.pointerDown(el, { button: 0, ...mods });
      fireEvent.pointerUp(el, { button: 0, ...mods });
      fireEvent.click(el, mods);
    };
    press(deviceEl("Synth").querySelector("header")!);
    press(deviceEl("Compressor").querySelector("header")!, { shiftKey: true });
    const both = [deviceOf("Keys", "Synth").id, deviceOf("Keys", "Compressor").id];
    expect(useDeviceClipboard.getState().selected).toEqual(both);
    // The power button of one of them: both off, still both selected.
    press(deviceEl("Compressor").querySelector(".eth-device__power")!);
    await flush();
    expect([deviceOf("Keys", "Synth").enabled, deviceOf("Keys", "Compressor").enabled]).toEqual([false, false]);
    expect(useDeviceClipboard.getState().selected).toEqual(both);
    press(deviceEl("Synth").querySelector(".eth-device__power")!);
    await flush();
    expect([deviceOf("Keys", "Synth").enabled, deviceOf("Keys", "Compressor").enabled]).toEqual([true, true]);
    // The header's menu: delete takes both.
    fireEvent.contextMenu(deviceEl("Synth").querySelector("header")!);
    const del = (useContextMenuStore.getState().menu?.items ?? []).find((e): e is ContextMenuItem => e !== "separator" && e.label === "delete device");
    await act(async () => del!.onSelect());
    await flush();
    expect(chainNames("Keys")).toEqual([]);
    await act(() => mock!.send(cmd("Edit", { type: "Undo" })));
    await flush();
    expect(chainNames("Keys")).toEqual(["Synth", "Compressor"]);
    // A plain click on the header's own space: that card alone.
    act(() => useDeviceClipboard.setState({ selected: both }));
    press(deviceEl("Synth").querySelector(".eth-device__name")!);
    expect(useDeviceClipboard.getState().selected).toEqual([deviceOf("Keys", "Synth").id]);
  });

  it("⌘X dims the cards until pasted; Delete removes the selected cards", async () => {
    await renderStack();
    fireEvent.pointerDown(deviceEl("Compressor").querySelector("header")!, { button: 0 });
    act(() => void document.dispatchEvent(new Event("cut")));
    expect(deviceEl("Compressor")).toHaveClass("eth-device--cut");
    expect(chainNames("Keys")).toEqual(["Synth", "Compressor"]);
    fireEvent.keyDown(deviceEl("Compressor"), { key: "Delete" });
    await flush();
    expect(chainNames("Keys")).toEqual(["Synth"]);
  });

  it("⌘V with the focus on no card pastes at the end of the selected track's chain", async () => {
    await renderStack();
    act(() => useSelectionStore.getState().selectTrack(trackByName("Keys").id));
    fireEvent.pointerDown(deviceEl("Compressor").querySelector("header")!, { button: 0 });
    act(() => void document.dispatchEvent(new Event("copy")));
    act(() => (document.activeElement as HTMLElement | null)?.blur());
    expect(document.activeElement).toBe(document.body);
    act(() => void document.dispatchEvent(new Event("paste")));
    await flush();
    expect(chainNames("Keys")).toEqual(["Synth", "Compressor", "Compressor"]);
  });

  it("⌘A / Ctrl+A on a card selects every device of its chain", async () => {
    await renderStack();
    fireEvent.pointerDown(deviceEl("Compressor").querySelector("header")!, { button: 0 });
    fireEvent.keyDown(deviceEl("Compressor"), { key: "a", code: "KeyA", ctrlKey: true });
    expect(deviceEl("Synth")).toHaveClass("eth-device--selected");
    expect(deviceEl("Compressor")).toHaveClass("eth-device--selected");
  });

  it("labels the sections; stacked cards have no up/down buttons (drag the header)", async () => {
    await renderStack();
    // Every section a MIDI track holds, empty ones included.
    expect(screen.getAllByRole("group").map((g) => g.getAttribute("aria-label"))).toEqual(["midi effects", "instrument", "audio effects"]);

    expect(screen.queryByRole("button", { name: /^move Synth (up|down)$/ })).toBeNull();
    expect(screen.getByRole("button", { name: "remove Synth" })).toBeInTheDocument();
  });

  it("header: the window toggle and the parameters toggle are independent", async () => {
    await renderStack();
    const card = deviceEl("Synth");
    const windowToggle = within(card).getByRole("button", { name: "open Synth window" });
    const paramsToggle = within(card).getByRole("button", { name: "show Synth parameters" });
    // No chevron any more; both start off.
    expect(screen.queryByRole("button", { name: /^(Expand|Collapse) / })).toBeNull();
    expect(windowToggle).toHaveAttribute("aria-pressed", "false");
    expect(paramsToggle).toHaveAttribute("aria-pressed", "false");
    expect(paramsToggle).toHaveAttribute("aria-expanded", "false");
    expect(paramsToggle).toHaveAttribute("title", "show parameters");
    expect(card).toHaveClass("eth-device--collapsed");
    expect(within(card).queryByTestId("device-params")).toBeNull();

    // The window: the factory UI renders there, never inline; the params stay hidden.
    fireEvent.click(windowToggle);
    expect(windowToggle).toHaveAttribute("aria-pressed", "true");
    const win = screen.getByRole("dialog", { name: "Synth window" });
    expect(await within(win).findByRole("slider", { name: "cutoff" })).toBeInTheDocument();
    expect(card.contains(win)).toBe(false);
    expect(within(card).queryByRole("slider", { name: "cutoff" })).toBeNull();
    expect(paramsToggle).toHaveAttribute("aria-pressed", "false");
    expect(within(card).queryByTestId("device-params")).toBeNull();

    // The params: the generic list, inline; the window stays open.
    fireEvent.click(paramsToggle);
    expect(paramsToggle).toHaveAttribute("aria-pressed", "true");
    expect(paramsToggle).toHaveAttribute("aria-expanded", "true");
    expect(paramsToggle).toHaveAttribute("title", "hide parameters");
    const params = within(card).getByTestId("device-params");
    expect(paramsToggle).toHaveAttribute("aria-controls", params.id);
    expect(within(params).getByRole("list", { name: "Synth parameters" })).toBeInTheDocument();
    expect(within(params).getByText("cutoff")).toBeInTheDocument();
    expect(windowToggle).toHaveAttribute("aria-pressed", "true");

    // Closing the window keeps the params; hiding the params keeps the window.
    fireEvent.click(within(win).getByRole("button", { name: "close Synth window" }));
    expect(screen.queryByRole("dialog", { name: "Synth window" })).toBeNull();
    expect(windowToggle).toHaveAttribute("aria-pressed", "false");
    expect(within(card).getByTestId("device-params")).toBeInTheDocument();
    fireEvent.click(windowToggle);
    fireEvent.click(paramsToggle);
    expect(within(card).queryByTestId("device-params")).toBeNull();
    expect(screen.getByRole("dialog", { name: "Synth window" })).toBeInTheDocument();

    // Other cards are untouched; the inspector hides every list when it shows a track.
    expect(deviceEl("Compressor")).toHaveClass("eth-device--collapsed");
    fireEvent.click(paramsToggle);
    act(() => collapseAll());
    expect(deviceEl("Synth")).toHaveClass("eth-device--collapsed");
    expect(screen.getByRole("dialog", { name: "Synth window" })).toBeInTheDocument();
  });

  it("a param changed in the inline list reaches the document; the list is capped and scrolls", async () => {
    await renderStack();
    fireEvent.click(screen.getByRole("button", { name: "show Synth parameters" }));
    const params = within(deviceEl("Synth")).getByTestId("device-params");
    expect(params).toHaveClass("eth-device-params");
    // The cap is a design token (CSS); the list is the scroll container inside it.
    expect(within(params).getByRole("list", { name: "Synth parameters" })).toHaveClass("eth-vlist");
    const attack = within(params).getByRole("slider", { name: "attack" });
    fireEvent.keyDown(attack, { key: "PageUp" });
    await flush();
    expect(deviceOf("Keys", "Synth").params[2]).toBeGreaterThan(5);
    // The window shows the same value.
    fireEvent.click(screen.getByRole("button", { name: "open Synth window" }));
    const inWindow = await within(screen.getByRole("dialog", { name: "Synth window" })).findByRole("slider", { name: "attack" });
    expect(inWindow).toHaveAttribute("aria-valuetext", attack.getAttribute("aria-valuetext"));
  });

  it("the window button opens the window; Escape on the window closes it; deleting the device closes it", async () => {
    await renderStack();
    const open = () => fireEvent.click(screen.getByRole("button", { name: "open Compressor window" }));
    open();
    const win = screen.getByRole("dialog", { name: "Compressor window" });
    expect(document.activeElement).toBe(win);
    fireEvent.keyDown(win, { key: "Escape" });
    expect(screen.queryByRole("dialog", { name: "Compressor window" })).toBeNull();
    // Several windows at once.
    open();
    fireEvent.click(screen.getByRole("button", { name: "open Synth window" }));
    expect(
      screen
        .getAllByRole("dialog")
        .map((d) => d.getAttribute("aria-label"))
        .sort(),
    ).toEqual(["Compressor window", "Synth window"]);
    // The last opened is on top (z-index; the DOM order stays put).
    const z = (name: string) => screen.getByRole("dialog", { name }).style.getPropertyValue("--fwin-stack");
    expect(Number(z("Synth window"))).toBeGreaterThan(Number(z("Compressor window")));
    fireEvent.pointerDown(screen.getByRole("dialog", { name: "Compressor window" }));
    expect(Number(z("Compressor window"))).toBeGreaterThan(Number(z("Synth window")));
    fireEvent.click(screen.getByRole("button", { name: "remove Compressor" }));
    await flush();
    expect(screen.getAllByRole("dialog").map((d) => d.getAttribute("aria-label"))).toEqual(["Synth window"]);
    // Undo brings the device back, its window stays closed.
    await act(() => mock!.send(cmd("Edit", { type: "Undo" })));
    expect(chainNames("Keys")).toEqual(["Synth", "Compressor"]);
    expect(screen.queryByRole("dialog", { name: "Compressor window" })).toBeNull();
  });

  it("a Drum Rack's window opens from its button (and again after closing)", async () => {
    await renderStack();
    const keys = trackByName("Keys");
    await act(() =>
      mock!.send(
        cmd("Device", {
          type: "Insert",
          id: "01DRUMWINRACK0000000000000",
          track: keys.id,
          device: { type: "Builtin", device: { type: "DrumRack" } },
          before: null,
        }),
      ),
    );
    await flush();
    const open = () => fireEvent.click(screen.getByRole("button", { name: /^open drum rack window$/i }));
    open();
    await flush();
    const win = await screen.findByRole("dialog", { name: /drum rack window/i });
    fireEvent.click(within(win).getByRole("button", { name: /close drum rack window/i }));
    expect(screen.queryByRole("dialog", { name: /drum rack window/i })).toBeNull();
    open();
    const rackWin = await screen.findByRole("dialog", { name: /drum rack window/i });
    expect(rackWin).toBeInTheDocument();

    // A pad's instrument (a Sampler) opens its own window from the button on its card.
    await act(() =>
      mock!.send(cmd("DrumRack", { type: "AddPad", id: "01DRUMWINPAD00000000000000", rack: "01DRUMWINRACK0000000000000", note: 36, name: "Kick" })),
    );
    await act(() =>
      mock!.send(
        cmd("DrumRack", {
          type: "InsertDevice",
          id: "01DRUMWINSAMPLER0000000000",
          pad: "01DRUMWINPAD00000000000000",
          device: { type: "Builtin", device: { type: "Sampler", sample: null, slices: { enabled: false, base_note: 36, markers: [] } } },
          before: null,
        }),
      ),
    );
    await flush();
    fireEvent.click(within(rackWin).getByRole("button", { name: /^open sampler window$/i }));
    await flush();
    expect(await screen.findByRole("dialog", { name: /sampler window/i })).toBeInTheDocument();
    // Still there once its sample picker has opened by itself (no sample yet).
    await act(() => new Promise((r) => setTimeout(r, 400)));
    expect(screen.getByRole("dialog", { name: /sampler window/i })).toBeInTheDocument();
    expect(screen.getByRole("dialog", { name: /drum rack window/i })).toBeInTheDocument();
  });

  it("copies a device (click, then ⌘C) and pastes it after the selected one (⌘V), one undo step", async () => {
    await renderStack();
    const select = (name: string) => fireEvent.pointerDown(deviceEl(name).querySelector("header")!, { button: 0 });
    select("Compressor");
    expect(deviceEl("Compressor")).toHaveClass("eth-device--selected");
    expect(document.activeElement).toBe(deviceEl("Compressor"));
    act(() => void document.dispatchEvent(new Event("copy")));
    act(() => void document.dispatchEvent(new Event("paste")));
    await flush();
    expect(chainNames("Keys")).toEqual(["Synth", "Compressor", "Compressor"]);
    // A different kind goes to its section: the copied compressor pasted "after" the synth
    // lands after the synth (audio effects), not before it.
    select("Synth");
    act(() => void document.dispatchEvent(new Event("paste")));
    await flush();
    expect(chainNames("Keys")).toEqual(["Synth", "Compressor", "Compressor", "Compressor"]);
    await act(() => mock!.send(cmd("Edit", { type: "Undo" })));
    expect(chainNames("Keys")).toEqual(["Synth", "Compressor", "Compressor"]);
  });

  it("only the window button opens a device's window: a click or Enter on the card does nothing", async () => {
    await renderStack();
    const card = deviceEl("Compressor");
    fireEvent.click(card.querySelector("header")!);
    fireEvent.click(card);
    fireEvent.keyDown(card, { key: "Enter" });
    expect(screen.queryByRole("dialog", { name: "Compressor window" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "open Compressor window" }));
    expect(screen.getByRole("dialog", { name: "Compressor window" })).toBeInTheDocument();
  });

  it("remembers each window's position (dragged by its title bar) and which windows are open", async () => {
    await renderStack();
    fireEvent.click(screen.getByRole("button", { name: "open Synth window" }));
    const win = screen.getByRole("dialog", { name: "Synth window" });
    const bar = win.querySelector("header")!;
    const x0 = parseFloat(win.style.left);
    const y0 = parseFloat(win.style.top);
    fireEvent.pointerDown(bar, { button: 0, clientX: 100, clientY: 100, pointerId: 1 });
    fireEvent.pointerMove(bar, { clientX: 140, clientY: 130, pointerId: 1 });
    fireEvent.pointerUp(bar, { clientX: 140, clientY: 130, pointerId: 1 });
    expect(parseFloat(win.style.left)).toBe(x0 + 40);
    expect(parseFloat(win.style.top)).toBe(y0 + 30);
    const saved = JSON.parse(localStorage.getItem("eth.devices.windows")!) as { open: string[] };
    const id = deviceOf("Keys", "Synth").id;
    expect(saved.open).toEqual([id]);
    // The geometry: the kit's floating windows, per window id.
    const rects = JSON.parse(localStorage.getItem("eth.windows")!) as Record<string, { x: number; y: number }>;
    expect(rects[`device:${id}`]).toEqual({ x: x0 + 40, y: y0 + 30 });
    // Closed and opened again: where it was.
    fireEvent.click(within(win).getByRole("button", { name: "close Synth window" }));
    fireEvent.click(screen.getByRole("button", { name: "open Synth window" }));
    expect(parseFloat(screen.getByRole("dialog", { name: "Synth window" }).style.left)).toBe(x0 + 40);
  });

  it("the row layout has the same header toggles and keeps left/right moves", async () => {
    await renderChain();
    expect(screen.queryByRole("button", { name: /Collapse / })).toBeNull();
    expect(screen.getByRole("button", { name: "open Synth window" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "show Synth parameters" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "move Synth right" })).toBeInTheDocument();
  });
});
