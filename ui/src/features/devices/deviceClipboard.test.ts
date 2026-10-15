// #582: a device paste is all or nothing, and racks with chains paste onto other tracks.
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { BuiltinDevice, Device, DeviceCategory, DeviceId, Project, TrackId, TrackKind } from "@/generated";
import { useNotices } from "@/features/notifications";
import { useProjectStore } from "@/state";
import { cmd, MockTransport, newId } from "@/transport";
import { pasteDevice, useDeviceClipboard } from "./deviceClipboard";

let mock: MockTransport;
const store = () => useProjectStore.getState();
const project = (): Project => store().project!;

const CATEGORY: Record<string, DeviceCategory> = { Synth: "Instrument", PolySynth: "Instrument", InstrumentRack: "Instrument" };
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
  useDeviceClipboard.setState({ selected: null, copied: null });
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
    expect(useNotices.getState().notices.at(-1)?.message).toMatch(/instruments can only be inserted on MIDI tracks/i);
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
    expect(useDeviceClipboard.getState().selected).toBe(copy!.id);
  });
});
