/** MockTransport: `Rack` / `Modulation` (v0.2). */
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
    expect(project().rack_chains[chain]?.name).toBe("chain 1");
    const synth = id();
    await send(cmd("Rack", { type: "InsertDevice", id: synth, chain, device: { type: "Builtin", device: { type: "Synth" } }, before: null }));
    expect(project().devices[synth]?.chain).toBe(chain);
    await send(cmd("Rack", { type: "SetChainMix", id: chain, volume: -6, pan: null, mute: true, solo: null }));
    expect(project().rack_chains[chain]).toMatchObject({ volume: -6, mute: true });
    await expect(send(cmd("Rack", { type: "SetChainZones", id: chain, keys: { lo: 70, hi: 60 }, velocities: null, select: null }))).rejects.toMatchObject({
      code: "InvalidArgument",
    });
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
    expect(project().modulators[lfo]?.name).toBe("lfo");
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

  describe("nested racks", () => {
    const addChain = async (rack: string) => {
      const c = id();
      await send(cmd("Rack", { type: "AddChain", id: c, rack, name: null, before: null }));
      return c;
    };
    const inChain = async (chain: string, type: string) => {
      const d = id();
      await send(cmd("Rack", { type: "InsertDevice", id: d, chain, device: { type: "Builtin", device: { type } as never }, before: null }));
      return d;
    };
    /** An effect rack's first chain (it comes with one). */
    const firstChain = (rack: string) => Object.values(project().rack_chains).find((c) => c.rack === rack)!.id;
    const rejected = (c: Command) => expect(send(c)).rejects.toMatchObject({ code: "InvalidArgument" });
    const insertIn = (chain: string, type: string): Command =>
      cmd("Rack", { type: "InsertDevice", id: id(), chain, device: { type: "Builtin", device: { type } as never }, before: null });

    it("nest by chain rules, never inside themselves, drum racks on instrument rack chains only, at most 6 levels", async () => {
      const track = await midiTrack();
      const outer = await insert(track, "InstrumentRack");
      const oc = await addChain(outer);
      const fx = await inChain(oc, "AudioEffectRack");
      const fc = firstChain(fx);
      // Audio effect rack chains take audio effects only (audio effect racks included).
      await rejected(insertIn(fc, "InstrumentRack"));
      await rejected(insertIn(fc, "DrumRack"));
      await rejected(cmd("Rack", { type: "MoveDevice", id: outer, chain: fc, before: null }));
      // Levels 3..6 fit, a 7th doesn't.
      let chain = fc;
      for (let level = 3; level <= 6; level++) chain = firstChain(await inChain(chain, "AudioEffectRack"));
      await rejected(insertIn(chain, "AudioEffectRack"));
      const delay = await inChain(chain, "Delay");
      expect(project().devices[delay]?.chain).toBe(chain);
    });

    it("macros reach any depth; a move drops what its new place puts out of reach (undoable)", async () => {
      const track = await midiTrack();
      const outer = await insert(track, "AudioEffectRack");
      const oc = await addChain(outer);
      const inner = await inChain(oc, "AudioEffectRack");
      const ic = await addChain(inner);
      const delay = await inChain(ic, "Delay");
      const fromOuter = id();
      const fromInner = id();
      await send(cmd("Modulation", { type: "Map", id: fromOuter, source: { type: "Macro", rack: outer, index: 0 }, device: delay, param: 0, depth: 0.5 }));
      await send(cmd("Modulation", { type: "Map", id: fromInner, source: { type: "Macro", rack: inner, index: 0 }, device: delay, param: 0, depth: 0.5 }));
      await send(cmd("Rack", { type: "MoveDevice", id: inner, chain: null, before: null }));
      expect(Object.keys(project().mod_mappings)).toEqual([fromInner]);
      await send(cmd("Edit", { type: "Undo" }));
      expect(project().devices[inner]?.chain).toBe(oc);
      expect(Object.keys(project().mod_mappings).sort()).toEqual([fromOuter, fromInner].sort());
    });

    it("duplicates and deletes nested racks with their chains", async () => {
      const track = await midiTrack();
      const outer = await insert(track, "AudioEffectRack");
      const inner = await inChain(firstChain(outer), "AudioEffectRack");
      await inChain(firstChain(inner), "Delay");
      const count = () => Object.values(project().devices).filter((d) => d.track === track).length;
      expect(count()).toBe(3);
      await send(cmd("Device", { type: "Duplicate", id: outer, new_id: id() }));
      expect(count()).toBe(6);
      expect(Object.keys(project().rack_chains)).toHaveLength(4);
      await send(cmd("Device", { type: "Remove", id: outer }));
      expect(count()).toBe(3);
      expect(Object.keys(project().rack_chains)).toHaveLength(2);
      // Removing the chain holding a rack takes the rack's own chains too.
      const copy = Object.values(project().devices).find((d) => d.track === track && d.chain == null)!.id;
      const chain = Object.values(project().rack_chains).find((c) => c.rack === copy)!.id;
      await send(cmd("Rack", { type: "RemoveChain", id: chain }));
      expect(Object.keys(project().rack_chains)).toHaveLength(0);
      expect(count()).toBe(1);
    });
  });

  it("a fresh audio / MIDI effect rack comes with chain 1 (one undo step); an instrument rack, a duplicate or a group don't", async () => {
    const track = await midiTrack();
    const chainsOf = (rack: string) => Object.values(project().rack_chains).filter((c) => c.rack === rack);
    const fx = await insert(track, "AudioEffectRack");
    expect(chainsOf(fx).map((c) => c.name)).toEqual(["chain 1"]);
    expect(project().devices[fx]?.name).toBe("audio effect rack");
    await send(cmd("Edit", { type: "Undo" }));
    expect(project().devices[fx]).toBeUndefined();
    expect(Object.keys(project().rack_chains)).toHaveLength(0);
    await send(cmd("Edit", { type: "Redo" }));
    expect(chainsOf(fx)).toHaveLength(1);
    const midi = await insert(track, "MidiEffectRack");
    expect(chainsOf(midi)).toHaveLength(1);
    const inst = await insert(track, "InstrumentRack");
    expect(chainsOf(inst)).toHaveLength(0);
    // On a rack chain too, with the lowercase default name.
    const ic = id();
    await send(cmd("Rack", { type: "AddChain", id: ic, rack: inst, name: null, before: null }));
    const nested = id();
    await send(
      cmd("Rack", { type: "InsertDevice", id: nested, chain: ic, device: { type: "Builtin", device: { type: "AudioEffectRack" } as never }, before: null }),
    );
    expect(chainsOf(nested)).toHaveLength(1);
    const gate = id();
    await send(
      cmd("Rack", {
        type: "InsertDevice",
        id: gate,
        chain: chainsOf(nested)[0]!.id,
        device: { type: "Builtin", device: { type: "Delay" } as never },
        before: null,
      }),
    );
    expect(project().devices[gate]?.name).toBe("delay");
    // A duplicate copies exactly its chains.
    const copy = id();
    await send(cmd("Device", { type: "Duplicate", id: fx, new_id: copy }));
    expect(chainsOf(copy)).toHaveLength(1);
  });

  it("a drum rack plays on an instrument rack chain: pads, duplicate, removal", async () => {
    const track = await midiTrack();
    const inst = await insert(track, "InstrumentRack");
    const ic = id();
    await send(cmd("Rack", { type: "AddChain", id: ic, rack: inst, name: null, before: null }));
    const drums = id();
    await send(cmd("Rack", { type: "InsertDevice", id: drums, chain: ic, device: { type: "Builtin", device: { type: "DrumRack" } as never }, before: null }));
    expect(project().devices[drums]?.chain).toBe(ic);
    const pad = id();
    await send(cmd("DrumRack", { type: "AddPad", id: pad, rack: drums, note: 36, name: null }));
    const synth = id();
    await send(cmd("DrumRack", { type: "InsertDevice", id: synth, pad, device: { type: "Builtin", device: { type: "Synth" } as never }, before: null }));
    // No drum rack under a pad (directly or in a pad rack's chain).
    const padRack = id();
    await send(
      cmd("DrumRack", { type: "InsertDevice", id: padRack, pad, device: { type: "Builtin", device: { type: "InstrumentRack" } as never }, before: null }),
    );
    const pc = id();
    await send(cmd("Rack", { type: "AddChain", id: pc, rack: padRack, name: null, before: null }));
    await expect(
      send(cmd("Rack", { type: "InsertDevice", id: id(), chain: pc, device: { type: "Builtin", device: { type: "DrumRack" } as never }, before: null })),
    ).rejects.toMatchObject({ code: "InvalidArgument" });
    // The instrument rack (holding a drum rack) can't move onto a pad.
    const top = await insert(track, "DrumRack");
    const topPad = id();
    await send(cmd("DrumRack", { type: "AddPad", id: topPad, rack: top, note: 36, name: null }));
    await expect(send(cmd("DrumRack", { type: "MoveDevice", id: inst, pad: topPad, before: null }))).rejects.toMatchObject({ code: "InvalidArgument" });
    // Duplicating the instrument rack copies the drum rack with its pads and pad chains.
    const pads = Object.keys(project().drum_pads).length;
    const devices = Object.keys(project().devices).length;
    await send(cmd("Device", { type: "Duplicate", id: inst, new_id: id() }));
    expect(Object.keys(project().drum_pads)).toHaveLength(pads + 1);
    expect(Object.keys(project().devices)).toHaveLength(devices + 4);
    await send(cmd("Edit", { type: "Undo" }));
    // Removing the chain takes the drum rack, its pads and their devices.
    await send(cmd("Rack", { type: "RemoveChain", id: ic }));
    expect(project().devices[drums]).toBeUndefined();
    expect(project().drum_pads[pad]).toBeUndefined();
    expect(project().devices[synth]).toBeUndefined();
    expect(project().devices[padRack]).toBeUndefined();
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
