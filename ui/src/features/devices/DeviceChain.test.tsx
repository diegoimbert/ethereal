import { act, cleanup, fireEvent, screen, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import type { Device } from "@/generated";
import { devicesOfTrack, useProjectStore, useSelectionStore } from "@/state";
import { cmd, type MockTransport } from "@/transport";
import { dragUp, flush, renderWithMock, resetStores, store, stubPointerCapture, trackByName } from "@/features/mixer/testUtils";
import { groupParams, insertableTypes, splitMainParams } from "./chainUtils";
import { BUILTIN_DESCRIPTORS } from "@/transport";
import { DeviceChain } from "./index";
import { resetCollapsed } from "./collapsed";
import { pickOption } from "@/kit/testing";
import { BROWSER_DRAG_MIME, type BrowserDragPayload } from "@/features/browser/dragPayload";

let mock: MockTransport | undefined;
beforeAll(stubPointerCapture);
afterEach(() => {
  resetStores(mock);
  mock = undefined;
});

const chainNames = (track: string) => devicesOfTrack(store().project!, trackByName(track).id).map((d) => d.name);
const deviceOf = (track: string, name: string): Device =>
  devicesOfTrack(store().project!, trackByName(track).id).find((d) => d.name === name)!;
const deviceEl = (name: string) => screen.getByRole("region", { name });

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
  mock = await renderWithMock(<DeviceChain />);
  // Descriptors load asynchronously.
  await screen.findAllByRole("slider", { name: "Cutoff" });
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
    expect(more.map((g) => g.group)).toEqual(["Envelope", "Filter", "Output"]);
    const small = groupParams(BUILTIN_DESCRIPTORS.Delay.params);
    expect(splitMainParams(small).more).toEqual([]);
  });

  it("groups visible params by section", () => {
    const groups = groupParams(BUILTIN_DESCRIPTORS.Synth.params);
    expect(groups.map((g) => g.group)).toEqual(["Oscillator", "Envelope", "Filter", "Output"]);
    expect(groupParams([{ ...BUILTIN_DESCRIPTORS.Delay.params[0]!, hidden: true }])).toEqual([]);
  });
});

describe("DeviceChain", () => {
  it("shows the selected track's chain (first track by default) with params from the descriptor", async () => {
    await renderChain();
    expect(screen.getByRole("combobox", { name: "Track" })).toHaveTextContent("Keys");
    const list = screen.getByRole("list", { name: "Keys devices" });
    expect(within(list).getAllByRole("region").map((r) => r.getAttribute("aria-label"))).toEqual(["Synth", "Compressor"]);
    const synth = deviceEl("Synth");
    // The Synth's declared layout: the oscillator's shape choice (4 labels → select) and the
    // filter curve's cutoff knob showing the formatted plain value.
    expect(within(synth).getByRole("combobox", { name: "Waveform" })).toHaveTextContent("Saw");
    expect(within(synth).getByRole("slider", { name: "Cutoff" })).toHaveAttribute("aria-valuetext", "2.4 kHz");

    act(() => useSelectionStore.getState().selectTrack(trackByName("Drums").id));
    expect(screen.getByRole("list", { name: "Drums devices" })).toBeInTheDocument();
    expect(screen.getByRole("combobox", { name: "Track" })).toHaveTextContent("Drums");
  });

  it("toggles bypass", async () => {
    await renderChain();
    fireEvent.click(screen.getByRole("button", { name: "Bypass Compressor" }));
    await flush();
    expect(deviceOf("Keys", "Compressor").enabled).toBe(false);
    expect(deviceEl("Compressor")).toHaveClass("eth-device--bypassed");
    fireEvent.click(screen.getByRole("button", { name: "Enable Compressor" }));
    await flush();
    expect(deviceOf("Keys", "Compressor").enabled).toBe(true);
  });

  it("adds, reorders and removes devices", async () => {
    await renderChain();
    pickOption(screen.getByRole("combobox", { name: "Add device" }), { value: "Delay" });
    await flush();
    expect(chainNames("Keys")).toEqual(["Synth", "Compressor", "Delay"]);

    fireEvent.click(screen.getByRole("button", { name: "Move Delay left" }));
    await flush();
    expect(chainNames("Keys")).toEqual(["Synth", "Delay", "Compressor"]);

    fireEvent.click(screen.getByRole("button", { name: "Move Synth right" }));
    await flush();
    expect(chainNames("Keys")).toEqual(["Delay", "Synth", "Compressor"]);
    expect(screen.getByRole("button", { name: "Move Compressor right" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Move Delay left" })).toBeDisabled();

    fireEvent.click(screen.getByRole("button", { name: "Remove Delay" }));
    await flush();
    expect(chainNames("Keys")).toEqual(["Synth", "Compressor"]);
  });

  it("inserts instruments at the start of the chain", async () => {
    await renderChain();
    pickOption(screen.getByRole("combobox", { name: "Add device" }), { value: "Sampler" });
    await flush();
    expect(chainNames("Keys")[0]).toBe("Sampler");
  });

  it("loads a sample dropped from the browser into a Sampler (one undo step)", async () => {
    await renderChain();
    pickOption(screen.getByRole("combobox", { name: "Add device" }), { value: "Sampler" });
    await flush();
    const slot = within(deviceEl("Sampler")).getByTestId("sample-slot");
    expect(slot.textContent).toMatch(/drop a sample/i);
    const payload = writePayload("Drums/Kick.wav");
    await act(async () => {
      fireEvent.dragOver(slot, { dataTransfer: payload });
      fireEvent.drop(slot, { dataTransfer: payload });
    });
    await flush();
    const sampler = deviceOf("Keys", "Sampler");
    const kind = sampler.kind.type === "Builtin" ? sampler.kind.device : null;
    const media = kind?.type === "Sampler" ? kind.sample : null;
    expect(media).not.toBeNull();
    expect(store().project!.media[media!]!.name).toMatch(/Kick/);
    expect(slot.textContent).toMatch(/Kick/);
    await act(async () => {
      await mock!.send(cmd("Edit", { type: "Undo" }));
    });
    const after = deviceOf("Keys", "Sampler").kind;
    expect(after.type === "Builtin" && after.device.type === "Sampler" ? after.device.sample : "?").toBeNull();
  });

  it("a knob drag is one undo step and maps through the param scale", async () => {
    await renderChain();
    const before = deviceOf("Keys", "Synth").params[6] ?? 8000;
    const undoLabel = store().history.undo_label;
    await dragUp(within(deviceEl("Synth")).getByRole("slider", { name: "Cutoff" }), 3, 10);
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
    pickOption(within(deviceEl("Synth")).getByRole("combobox", { name: "Waveform" }), { value: "2" });
    await flush();
    expect(deviceOf("Keys", "Synth").params[0]).toBe(2);

    // The envelope widget's attack control (param 2 in `ether_devices::synth`).
    const attack = within(deviceEl("Synth")).getByRole("slider", { name: "Attack" });
    fireEvent.keyDown(attack, { key: "PageUp" });
    await flush();
    expect(deviceOf("Keys", "Synth").params[2]).toBeGreaterThan(5);
    fireEvent.doubleClick(attack);
    await flush();
    expect(deviceOf("Keys", "Synth").params[2]).toBeCloseTo(5, 9);  });

  it("moves a dragged device before the drop target", async () => {
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
    expect(chainNames("Keys")).toEqual(["Compressor", "Synth"]);
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
    mock = await renderWithMock(<KeysStack />);
    await screen.findAllByRole("slider", { name: "Cutoff" });
  }

  it("shows the given track's chain as full-width cards, no track picker, add device at the bottom", async () => {
    await renderStack();
    expect(screen.queryByRole("combobox", { name: "Track" })).toBeNull();
    const list = screen.getByRole("list", { name: "Keys devices" });
    expect(list).toHaveClass("eth-devices__chain--stack");
    expect(within(list).getAllByRole("region").map((r) => r.getAttribute("aria-label"))).toEqual(["Synth", "Compressor"]);
    // The add-device select comes after the chain.
    const add = screen.getByRole("combobox", { name: "Add device" });
    expect(list.compareDocumentPosition(add) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("moves cards up/down", async () => {
    await renderStack();
    expect(screen.getByRole("button", { name: "Move Synth up" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Move Synth down" }));
    await flush();
    expect(chainNames("Keys")).toEqual(["Compressor", "Synth"]);
  });

  it("collapses a card (animated body) and remembers it per device", async () => {
    await renderStack();
    const toggle = screen.getByRole("button", { name: "Collapse Synth" });
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    fireEvent.click(toggle);
    const card = deviceEl("Synth");
    expect(card).toHaveClass("eth-device--collapsed");
    expect(card.querySelector(".eth-device__collapse")).toHaveClass("eth-device__collapse--closed");
    expect(screen.getByRole("button", { name: "Expand Synth" })).toHaveAttribute("aria-expanded", "false");
    // Other cards stay open; the state survives a remount.
    expect(deviceEl("Compressor")).not.toHaveClass("eth-device--collapsed");
    resetStores(mock);
    cleanup();
    await renderStack();
    expect(deviceEl("Synth")).toHaveClass("eth-device--collapsed");
    fireEvent.click(screen.getByRole("button", { name: "Expand Synth" }));
    expect(deviceEl("Synth")).not.toHaveClass("eth-device--collapsed");
  });

  it("the row layout has no collapse toggle and keeps left/right moves", async () => {
    await renderChain();
    expect(screen.queryByRole("button", { name: /Collapse / })).toBeNull();
    expect(screen.getByRole("button", { name: "Move Synth right" })).toBeInTheDocument();
  });
});
