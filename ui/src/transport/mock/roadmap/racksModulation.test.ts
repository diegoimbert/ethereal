/** MockTransport: `Rack` / `Modulation` (v0.2, racks-modulation). */
import { describe, expect, it } from "vitest";
import type { Command, Project } from "@/generated";
import { cmd } from "../../cmd";
import { useMock } from "./testUtils";

describe("MockTransport racks and modulation (racks-modulation)", () => {
  const f = useMock();
  const project = (): Project => f.mock.snapshot();
  const send = (c: Command) => f.mock.send(c);
  let n = 0;
  const id = () => `01J${String(++n).padStart(23, "0")}`;

  async function midiTrack(): Promise<string> {
    const track = id();
    await send(cmd("Track", { type: "Create", id: track, kind: "Midi", name: null, color: null, parent: null, before: null }));
    return track;
  }
  async function insert(track: string, device: string): Promise<string> {
    const d = id();
    await send(cmd("Device", { type: "Insert", id: d, track, device: { type: "Builtin", device: { type: device } as never }, before: null }));
    return d;
  }

  it("lists modulator kinds, including keytrack and velocity", async () => {
    const kinds = await send(cmd("Modulation", { type: "ListModulatorKinds" }));
    expect(kinds).toMatchObject({ type: "ModulatorKinds" });
    if (kinds.type !== "ModulatorKinds") throw new Error("kinds");
    expect(kinds.kinds.map((k) => k.kind)).toEqual(["Lfo", "Envelope", "EnvelopeFollower", "Steps", "Random", "Keytrack", "Velocity"]);
  });

  it("builds a rack with chains, chain devices and a macro mapping, and cascades on delete", async () => {
    const track = await midiTrack();
    const rack = await insert(track, "InstrumentRack");
    const chain = id();
    await send(cmd("Rack", { type: "AddChain", id: chain, rack, name: null, before: null }));
    expect(project().rack_chains[chain]?.name).toBe("Chain 1");
    const synth = id();
    await send(cmd("Rack", { type: "InsertDevice", id: synth, chain, device: { type: "Builtin", device: { type: "Synth" } }, before: null }));
    expect(project().devices[synth]?.chain).toBe(chain);
    await send(cmd("Rack", { type: "SetChainMix", id: chain, volume: -6, pan: null, mute: true, solo: null }));
    expect(project().rack_chains[chain]).toMatchObject({ volume: -6, mute: true });
    await expect(
      send(cmd("Rack", { type: "SetChainZones", id: chain, keys: { lo: 70, hi: 60 }, velocities: null, select: null })),
    ).rejects.toMatchObject({ code: "InvalidArgument" });
    const map = id();
    await send(cmd("Modulation", { type: "Map", id: map, source: { type: "Macro", rack, index: 2 }, device: synth, param: 1, depth: 2 }));
    expect(project().mod_mappings[map]?.depth).toBe(1);
    // Duplicate (source, target) is rejected.
    await expect(
      send(cmd("Modulation", { type: "Map", id: id(), source: { type: "Macro", rack, index: 2 }, device: synth, param: 1, depth: 0.5 })),
    ).rejects.toMatchObject({ code: "InvalidArgument" });
    await send(cmd("Device", { type: "Remove", id: rack }));
    expect(Object.keys(project().rack_chains)).toHaveLength(0);
    expect(project().devices[synth]).toBeUndefined();
    expect(Object.keys(project().mod_mappings)).toHaveLength(0);
  });

  it("enforces chain content rules and modulation scope", async () => {
    const track = await midiTrack();
    const fx = await insert(track, "AudioEffectRack");
    const chain = id();
    await send(cmd("Rack", { type: "AddChain", id: chain, rack: fx, name: "Wet", before: null }));
    await expect(
      send(cmd("Rack", { type: "InsertDevice", id: id(), chain, device: { type: "Builtin", device: { type: "Synth" } }, before: null })),
    ).rejects.toMatchObject({ code: "InvalidArgument" });
    const delay = await insert(track, "Delay");
    const lfo = id();
    await send(cmd("Modulation", { type: "AddModulator", id: lfo, device: fx, kind: "Lfo", name: null }));
    expect(project().modulators[lfo]?.name).toBe("LFO");
    // Out of the rack: rejected; on its own macros: rejected.
    await expect(
      send(cmd("Modulation", { type: "Map", id: id(), source: { type: "Modulator", modulator: lfo }, device: delay, param: 0, depth: 0.5 })),
    ).rejects.toMatchObject({ code: "InvalidArgument" });
    await expect(
      send(cmd("Modulation", { type: "Map", id: id(), source: { type: "Modulator", modulator: lfo }, device: fx, param: 0, depth: 0.5 })),
    ).rejects.toMatchObject({ code: "InvalidArgument" });
    // Move the delay into the rack: now in scope.
    await send(cmd("Rack", { type: "MoveDevice", id: delay, chain, before: null }));
    const map = id();
    await send(cmd("Modulation", { type: "Map", id: map, source: { type: "Modulator", modulator: lfo }, device: delay, param: 0, depth: 0.5 }));
    await send(cmd("Modulation", { type: "SetDepth", id: map, depth: -0.25 }));
    expect(project().mod_mappings[map]?.depth).toBe(-0.25);
    await send(cmd("Modulation", { type: "SetModulatorParam", modulator: lfo, param: 1, value: 1000 }));
    expect(project().modulators[lfo]?.params[1]).toBe(40);
    await send(cmd("Modulation", { type: "RemoveModulator", id: lfo }));
    expect(project().mod_mappings[map]).toBeUndefined();
  });

  it("groups devices into a rack, re-hosting their modulators", async () => {
    const track = await midiTrack();
    const a = await insert(track, "Delay");
    const b = await insert(track, "Reverb");
    const lfo = id();
    await send(cmd("Modulation", { type: "AddModulator", id: lfo, device: a, kind: "Lfo", name: null }));
    await send(cmd("Modulation", { type: "Map", id: id(), source: { type: "Modulator", modulator: lfo }, device: a, param: 0, depth: 0.5 }));
    const rack = id();
    const chain = id();
    await send(cmd("Rack", { type: "Group", rack, rack_type: "AudioEffectRack", chain, devices: [b, a] }));
    const p = project();
    expect(p.devices[a]?.chain).toBe(chain);
    expect(p.devices[b]?.chain).toBe(chain);
    const mods = Object.values(p.modulators);
    expect(mods).toHaveLength(1);
    expect(mods[0]!.device).toBe(rack);
    const maps = Object.values(p.mod_mappings);
    expect(maps).toHaveLength(1);
    expect(maps[0]!.source).toEqual({ type: "Modulator", modulator: mods[0]!.id });
    // One undo step.
    await send(cmd("Edit", { type: "Undo" }));
    expect(project().devices[rack]).toBeUndefined();
    expect(project().modulators[lfo]).toBeDefined();
  });
});
