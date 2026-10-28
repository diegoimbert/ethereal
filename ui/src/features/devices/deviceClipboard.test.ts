// A device paste is all or nothing, and racks with chains paste onto other tracks.
import { waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { BuiltinDevice, Device, DeviceCategory, DeviceId, Project, TrackId, TrackKind } from "@/generated";
import { useNotices } from "@/features/notifications";
import { useProjectStore } from "@/state";
import { cmd, newId } from "@/transport";
import { MockTransport } from "@/transport/mock";
import { useConfirm } from "@/kit/confirmStore";
import { deleteDevices, pasteDevice, pasteTarget, setDevicesEnabled, useDeviceClipboard } from "./deviceClipboard";

let mock: MockTransport;
const store = () => useProjectStore.getState();
const project = (): Project => store().project!;

const CATEGORY: Record<string, DeviceCategory> = {
  Synth: "Instrument",
  PolySynth: "Instrument",
  InstrumentRack: "Instrument",
  Arpeggiator: "NoteEffect",
  Chord: "NoteEffect",
};
const categoryOf = (d: Device): DeviceCategory => (d.kind.type === "Builtin" ? (CATEGORY[d.kind.device.type] ?? "AudioEffect") : "AudioEffect");

beforeEach(async () => {
  mock = new MockTransport({ timers: "manual", seed: 3 });
  mock.onEvent((e) => {
    if (e.type === "ProjectLoaded") store().loadProject(e.project);
    if (e.type === "Patch") store().applyPatch(e.patch);
  });
  store().loadProject((await mock.connect())!);
});
afterEach(() => {
  mock.dispose();
  store().reset();
  useDeviceClipboard.setState({ selected: [], anchor: null, copied: [], cut: false });
  useNotices.getState().clear();
});

async function track(kind: TrackKind): Promise<TrackId> {
  const id = newId();
  await mock.send(cmd("Track", { type: "Create", id, kind, name: null, color: null, parent: null, before: null }));
  return id;
}
async function insert(track: TrackId, type: string): Promise<DeviceId> {
  const id = newId();
  await mock.send(cmd("Device", { type: "Insert", id, track, device: { type: "Builtin", device: { type } as BuiltinDevice }, before: null }));
  return id;
}

describe("device paste (#582)", () => {
  it("a paste that can't land changes nothing: no stray copy, no undo step", async () => {
    const midi = await track("Midi");
    const synth = await insert(midi, "Synth");
    const audio = await track("Audio");
    const eq = await insert(audio, "Eq");
    const devices = Object.keys(project().devices).length;
    const undo = project() && store().history.undo_id;
    useDeviceClipboard.getState().copy(synth);
    await pasteDevice(mock, eq, categoryOf);
    expect(Object.keys(project().devices)).toHaveLength(devices);
    expect(store().history.undo_id).toBe(undo);
    expect(useNotices.getState().notices.at(-1)?.message).toMatch(/is an instrument: select a midi track/i);
  });

  it("an audio-effect rack with a chain pastes onto another audio track, its chains with it", async () => {
    const a = await track("Audio");
    const rack = await insert(a, "AudioEffectRack");
    const chain = newId();
    await mock.send(cmd("Rack", { type: "AddChain", id: chain, rack, name: null, before: null }));
    await mock.send(cmd("Rack", { type: "InsertDevice", id: newId(), chain, device: { type: "Builtin", device: { type: "Delay" } }, before: null }));
    const b = await track("Audio");
    const eq = await insert(b, "Eq");
    useDeviceClipboard.getState().copy(rack);
    await pasteDevice(mock, eq, categoryOf);
    const p = project();
    const copy = Object.values(p.devices).find((d) => d.track === b && d.kind.type === "Builtin" && d.kind.device.type === "AudioEffectRack");
    expect(copy).toBeDefined();
    // As many chains as the source (a new rack has its first chain), the Delay in one, on B.
    const chains = new Set(Object.values(p.rack_chains).flatMap((c) => (c.rack === copy!.id ? [c.id] : [])));
    expect(chains.size).toBe(Object.values(p.rack_chains).filter((c) => c.rack === rack).length);
    const inChain = Object.values(p.devices).filter((d) => d.chain != null && chains.has(d.chain));
    expect(inChain.map((d) => [d.track, d.kind.type === "Builtin" && d.kind.device.type])).toEqual([[b, "Delay"]]);
    // After the target, and the source track keeps only its own rack.
    const onB = Object.values(p.devices)
      .filter((d) => d.track === b && d.chain == null && d.pad === null)
      .sort((x, y) => (x.order < y.order ? -1 : 1));
    expect(onB.map((d) => d.id)).toEqual([eq, copy!.id]);
    expect(Object.values(p.devices).filter((d) => d.track === a && d.chain == null)).toHaveLength(1);
    expect(useDeviceClipboard.getState().selected).toEqual([copy!.id]);
  });
});

const onTrack = (track: TrackId) =>
  Object.values(project().devices)
    .filter((d) => d.track === track && d.chain == null && d.pad === null)
    .sort((x, y) => (x.order < y.order ? -1 : 1));
const typeOf = (d: Device) => (d.kind.type === "Builtin" ? d.kind.device.type : "plugin");

describe("paste target: after the selection, else the chain's end", () => {
  it("with a device: right after it", async () => {
    const a = await track("Audio");
    const eq = await insert(a, "Eq");
    await insert(a, "Delay");
    useDeviceClipboard.getState().copy(eq);
    await pasteDevice(mock, eq, categoryOf);
    expect(onTrack(a).map(typeOf)).toEqual(["Eq", "Eq", "Delay"]);
  });

  it("with no device: at the end of the chain, and again after the copy", async () => {
    const a = await track("Audio");
    const eq = await insert(a, "Eq");
    await insert(a, "Delay");
    useDeviceClipboard.getState().copy(eq);
    await pasteDevice(mock, { type: "Track", track: a }, categoryOf);
    const first = onTrack(a).at(-1)!;
    expect(onTrack(a).map(typeOf)).toEqual(["Eq", "Delay", "Eq"]);
    expect(useDeviceClipboard.getState().selected).toEqual([first.id]);
    await pasteDevice(mock, { type: "Track", track: a }, categoryOf);
    expect(onTrack(a).map(typeOf)).toEqual(["Eq", "Delay", "Eq", "Eq"]);
  });

  it("an empty chain takes the paste", async () => {
    const a = await track("Audio");
    const eq = await insert(a, "Eq");
    const b = await track("Audio");
    useDeviceClipboard.getState().copy(eq);
    await pasteDevice(mock, { type: "Track", track: b }, categoryOf);
    expect(onTrack(b).map(typeOf)).toEqual(["Eq"]);
  });

  it("an instrument at the chain's end still goes before the audio effects", async () => {
    const m = await track("Midi");
    const synth = await insert(m, "Synth");
    const m2 = await track("Midi");
    await insert(m2, "Eq");
    useDeviceClipboard.getState().copy(synth);
    await pasteDevice(mock, { type: "Track", track: m2 }, categoryOf);
    expect(onTrack(m2).map(typeOf)).toEqual(["Synth", "Eq"]);
  });

  it("the end of a rack chain", async () => {
    const a = await track("Audio");
    const rack = await insert(a, "AudioEffectRack");
    const chain = newId();
    await mock.send(cmd("Rack", { type: "AddChain", id: chain, rack, name: null, before: null }));
    await mock.send(cmd("Rack", { type: "InsertDevice", id: newId(), chain, device: { type: "Builtin", device: { type: "Delay" } }, before: null }));
    const eq = await insert(a, "Eq");
    useDeviceClipboard.getState().copy(eq);
    await pasteDevice(mock, { type: "Chain", chain }, categoryOf);
    const inChain = Object.values(project().devices)
      .filter((d) => d.chain === chain)
      .sort((x, y) => (x.order < y.order ? -1 : 1));
    expect(inChain.map(typeOf)).toEqual(["Delay", "Eq"]);
    expect(onTrack(a).map(typeOf)).toEqual(["AudioEffectRack", "Eq"]);
  });
});

describe("pasteTarget: from the focus", () => {
  afterEach(() => {
    document.body.innerHTML = "";
  });
  const focus = (sel: string) => document.querySelector<HTMLElement>(sel)!.focus();

  it("a focused card, a focused chain (the innermost), nothing elsewhere or in a field", () => {
    document.body.innerHTML = `
      <div data-paste-chain='{"type":"Track","track":"t"}' tabindex="-1" id="chain">
        <section class="eth-device" data-device="d1" tabindex="-1" id="card">
          <div data-paste-chain='{"type":"Chain","chain":"c"}' tabindex="-1" id="inner"></div>
          <input id="field" />
        </section>
      </div>
      <button id="elsewhere">x</button>`;
    focus("#card");
    expect(pasteTarget()).toBe("d1");
    focus("#chain");
    expect(pasteTarget()).toEqual({ type: "Track", track: "t" });
    focus("#inner");
    expect(pasteTarget()).toEqual({ type: "Chain", chain: "c" });
    focus("#field");
    expect(pasteTarget()).toBeNull();
    focus("#elsewhere");
    expect(pasteTarget()).toBeNull();
  });
});

describe("several devices (MIDI effects, instruments and audio effects together)", () => {
  afterEach(() => useConfirm.getState().close());

  it("selects with ⌘ (toggle) and Shift (a range in the chain)", async () => {
    const m = await track("Midi");
    const [arp, synth, eq] = [await insert(m, "Arpeggiator"), await insert(m, "Synth"), await insert(m, "Eq")];
    const { select } = useDeviceClipboard.getState();
    select(arp);
    select(eq, "toggle");
    expect(useDeviceClipboard.getState().selected).toEqual([arp, eq]);
    select(arp, "toggle");
    expect(useDeviceClipboard.getState().selected).toEqual([eq]);
    // From the last clicked (the arpeggiator) to the eq: the cards between too.
    select(eq, "range");
    expect(useDeviceClipboard.getState().selected).toEqual([arp, synth, eq]);
  });

  it("pastes each copy in its own section, one undo step", async () => {
    const a = await track("Midi");
    const arp = await insert(a, "Arpeggiator");
    await insert(a, "Synth");
    const eq = await insert(a, "Eq");
    const b = await track("Midi");
    await insert(b, "Delay");
    useDeviceClipboard.getState().copy([eq, arp]);
    const undo = store().history.undo_id;
    await pasteDevice(mock, { type: "Track", track: b }, categoryOf);
    expect(onTrack(b).map(typeOf)).toEqual(["Arpeggiator", "Delay", "Eq"]);
    expect(useDeviceClipboard.getState().selected).toHaveLength(2);
    await mock.send(cmd("Edit", { type: "Undo" }));
    expect(onTrack(b).map(typeOf)).toEqual(["Delay"]);
    expect(store().history.undo_id).toBe(undo);
  });

  it("after a selected device: its kind right after it, the others at their section's end", async () => {
    const m = await track("Midi");
    const arp = await insert(m, "Arpeggiator");
    const chord = await insert(m, "Chord");
    await insert(m, "Eq");
    const delay = await insert(m, "Delay");
    useDeviceClipboard.getState().copy([delay, arp]);
    // After the arpeggiator: the copied arpeggiator right after it, the delay at the end.
    await pasteDevice(mock, arp, categoryOf);
    expect(onTrack(m).map(typeOf)).toEqual(["Arpeggiator", "Arpeggiator", "Chord", "Eq", "Delay", "Delay"]);
    expect(chord).toBeTruthy();
  });

  it("an instrument where the chain has one replaces it, once confirmed", async () => {
    const a = await track("Midi");
    const synth = await insert(a, "Synth");
    const b = await track("Midi");
    await insert(b, "PolySynth");
    await insert(b, "Eq");
    useDeviceClipboard.getState().copy(synth);
    await pasteDevice(mock, { type: "Track", track: b }, categoryOf);
    // Asked first: nothing changed yet.
    const request = useConfirm.getState().request;
    expect(request?.action).toBe("replace");
    expect(onTrack(b).map(typeOf)).toEqual(["PolySynth", "Eq"]);
    request!.run();
    await waitFor(() => expect(onTrack(b).map(typeOf)).toEqual(["Synth", "Eq"]));
    // One undo step brings the old one back.
    await mock.send(cmd("Edit", { type: "Undo" }));
    expect(onTrack(b).map(typeOf)).toEqual(["PolySynth", "Eq"]);
  });

  it("an audio track takes the audio effects and says why it skips the others", async () => {
    const m = await track("Midi");
    const arp = await insert(m, "Arpeggiator");
    const eq = await insert(m, "Eq");
    const a = await track("Audio");
    useDeviceClipboard.getState().copy([arp, eq]);
    await pasteDevice(mock, { type: "Track", track: a }, categoryOf);
    expect(onTrack(a).map(typeOf)).toEqual(["Eq"]);
    expect(useNotices.getState().notices.at(-1)?.message).toMatch(/is a midi effect: select a midi track/i);
  });
});

describe("cut and delete", () => {
  it("a cut moves the devices on paste (one undo step); the next paste copies", async () => {
    const a = await track("Midi");
    const arp = await insert(a, "Arpeggiator");
    await insert(a, "Synth");
    const eq = await insert(a, "Eq");
    const b = await track("Midi");
    await insert(b, "Delay");
    useDeviceClipboard.getState().copy([arp, eq], true);
    // Nothing moves until the paste.
    expect(onTrack(a).map(typeOf)).toEqual(["Arpeggiator", "Synth", "Eq"]);
    await pasteDevice(mock, { type: "Track", track: b }, categoryOf);
    expect(onTrack(b).map(typeOf)).toEqual(["Arpeggiator", "Delay", "Eq"]);
    expect(onTrack(a).map(typeOf)).toEqual(["Synth"]);
    expect(useDeviceClipboard.getState().cut).toBe(false);
    await mock.send(cmd("Edit", { type: "Undo" }));
    expect(onTrack(a).map(typeOf)).toEqual(["Arpeggiator", "Synth", "Eq"]);
    expect(onTrack(b).map(typeOf)).toEqual(["Delay"]);
  });

  it("deletes the selected devices in one undo step", async () => {
    const a = await track("Midi");
    const arp = await insert(a, "Arpeggiator");
    await insert(a, "Synth");
    const eq = await insert(a, "Eq");
    await deleteDevices(mock, [arp, eq]);
    expect(onTrack(a).map(typeOf)).toEqual(["Synth"]);
    await mock.send(cmd("Edit", { type: "Undo" }));
    expect(onTrack(a).map(typeOf)).toEqual(["Arpeggiator", "Synth", "Eq"]);
  });
});

describe("on / off", () => {
  it("a device of a selection turns the whole selection on or off, in one undo step; alone, just itself", async () => {
    const a = await track("Midi");
    const arp = await insert(a, "Arpeggiator");
    const synth = await insert(a, "Synth");
    const eq = await insert(a, "Eq");
    const enabled = () => [arp, synth, eq].map((d) => project().devices[d]!.enabled);
    useDeviceClipboard.setState({ selected: [arp, eq] });
    await setDevicesEnabled(mock, eq, false);
    expect(enabled()).toEqual([false, true, false]);
    await mock.send(cmd("Edit", { type: "Undo" }));
    expect(enabled()).toEqual([true, true, true]);
    // Not in the selection: only itself.
    await setDevicesEnabled(mock, synth, false);
    expect(enabled()).toEqual([true, false, true]);
  });
});

describe("device paste is the copy as it was (ux-session-4)", () => {
  it("a param, on/off or name changed after the copy doesn't follow into the paste", async () => {
    const audio = await track("Audio");
    const eq = await insert(audio, "Eq");
    // A param that takes both values (not clamped).
    let param = -1;
    for (const key of Object.keys(project().devices[eq]!.params)) {
      await mock.send(cmd("Device", { type: "SetParam", device: eq, param: Number(key), value: 5 }));
      if (project().devices[eq]!.params[Number(key)] !== 5) continue;
      param = Number(key);
      break;
    }
    expect(param).toBeGreaterThanOrEqual(0);
    await mock.send(cmd("Device", { type: "SetParam", device: eq, param, value: 2 }));
    const copied = project().devices[eq]!;
    useDeviceClipboard.getState().copy(eq);
    // Edited after the copy.
    await mock.send(cmd("Device", { type: "SetParam", device: eq, param, value: 5 }));
    await mock.send(cmd("Device", { type: "SetEnabled", id: eq, enabled: false }));
    await mock.send(cmd("Device", { type: "Rename", id: eq, name: "edited" }));
    await pasteDevice(mock, eq, categoryOf);
    const copy = project().devices[useDeviceClipboard.getState().selected[0]!]!;
    expect(copy.id).not.toBe(eq);
    expect(copy.params[param]).toBe(copied.params[param]);
    expect(copy.enabled).toBe(true);
    expect(copy.name).toBe(copied.name);
    // The source keeps its edits.
    expect(project().devices[eq]!.params[param]).toBe(5);
  });
});
