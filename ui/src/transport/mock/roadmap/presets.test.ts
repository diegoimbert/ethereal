/** MockTransport: `Preset` (v0.2, presets). */
import { describe, expect, it } from "vitest";
import type { BuiltinDevice, BuiltinDeviceType, Command, Device, PresetInfo, ReplyValue } from "@/generated";
import { deriveId } from "@/features/comping/model";
import { cmd } from "../../cmd";
import { deviceKey, fileStem, rackSnapshot } from "./presets";
import { createTrack, project, testId, undo, useMock } from "./testUtils";

const presetsOf = (r: ReplyValue): PresetInfo[] => (r.type === "Presets" ? r.presets : []);
const presetOf = (r: ReplyValue): PresetInfo => {
  if (r.type !== "Preset") throw new Error(r.type);
  return r.preset;
};

describe("MockTransport Preset (presets)", () => {
  const f = useMock();
  const synth = (): Device => Object.values(project(f).devices).find((d) => d.kind.type === "Builtin" && d.kind.device.type === "Synth")!;

  it("lists the embedded factory presets, sorted and filtered", async () => {
    const all = presetsOf(
      await f.mock.send(
        cmd("Preset", {
          type: "List",
          device: { type: "Builtin", device: "Synth" },
          text: null,
        }),
      ),
    );
    expect(all.map((p) => p.name)).toEqual(["Glass Keys", "Pluck", "Soft Pad", "Square Lead", "Sub Bass"]);
    expect(all.every((p) => p.preset.source === "Factory" && p.preset.id.startsWith("synth/"))).toBe(true);
    const bass = presetsOf(await f.mock.send(cmd("Preset", { type: "List", device: null, text: "BASS" })));
    expect(bass.map((p) => p.preset.id)).toContain("synth/sub-bass");
  });

  it("loads a preset as one undo step", async () => {
    const d = synth();
    await f.mock.send(
      cmd("Preset", {
        type: "Load",
        device: d.id,
        preset: { source: "Factory", id: "synth/soft-pad" },
      }),
    );
    expect(project(f).devices[d.id]!.params[2]).toBe(800);
    expect(project(f).devices[d.id]!.params[6]).toBe(2500);
    await undo(f);
    expect(project(f).devices[d.id]!.params).toEqual(d.params);
    await expect(
      f.mock.send(
        cmd("Preset", {
          type: "Load",
          device: d.id,
          preset: { source: "Factory", id: "compressor/gentle-glue" },
        }),
      ),
    ).rejects.toMatchObject({ code: "InvalidArgument" });
  });

  it("saves, renames, edits and deletes user presets", async () => {
    const d = synth();
    await f.mock.send(cmd("Device", { type: "SetParam", device: d.id, param: 6, value: 1234 }));
    const saved = presetOf(
      await f.mock.send(
        cmd("Preset", {
          type: "Save",
          device: d.id,
          name: " My Pad ",
          meta: { tags: ["Pad", "pad"], author: null, description: null },
          overwrite: false,
        }),
      ),
    );
    expect(saved).toMatchObject({
      name: "My Pad",
      preset: { source: "User", id: "synth/My Pad.etherpreset" },
      meta: { tags: ["pad"] },
    });
    expect(f.events.some((e) => e.type === "Preset")).toBe(true);
    await expect(
      f.mock.send(
        cmd("Preset", {
          type: "Save",
          device: d.id,
          name: "my pad",
          meta: { tags: [], author: null, description: null },
          overwrite: false,
        }),
      ),
    ).rejects.toMatchObject({ code: "InvalidArgument" });
    const renamed = presetOf(
      await f.mock.send(
        cmd("Preset", {
          type: "Rename",
          preset: saved.preset,
          name: "Dark Pad",
        }),
      ),
    );
    expect(renamed.preset.id).toBe("synth/Dark Pad.etherpreset");
    await f.mock.send(cmd("Device", { type: "SetParam", device: d.id, param: 6, value: 500 }));
    await f.mock.send(cmd("Preset", { type: "Load", device: d.id, preset: renamed.preset }));
    expect(project(f).devices[d.id]!.params[6]).toBe(1234);
    const list = presetsOf(
      await f.mock.send(
        cmd("Preset", {
          type: "List",
          device: { type: "Builtin", device: "Synth" },
          text: null,
        }),
      ),
    );
    expect(list.at(-1)?.name).toBe("Dark Pad");
    await f.mock.send(cmd("Preset", { type: "Delete", preset: renamed.preset }));
    await expect(f.mock.send(cmd("Preset", { type: "Delete", preset: renamed.preset }))).rejects.toMatchObject({ code: "NotFound" });
    await expect(
      f.mock.send(
        cmd("Preset", {
          type: "Delete",
          preset: { source: "Factory", id: "synth/pluck" },
        }),
      ),
    ).rejects.toMatchObject({ code: "InvalidArgument" });
  });

  it("names folders and files like the engine", () => {
    expect(deviceKey("PolySynth")).toBe("poly-synth");
    expect(deviceKey("Eq")).toBe("eq");
    expect(fileStem("a/b:c?")).toBe("a-b-c-");
    expect(fileStem("  ..hidden. ")).toBe("hidden");
    expect(fileStem("")).toBe("Preset");
  });
});

describe("MockTransport rack presets (rack-presets)", () => {
  const f = useMock();
  const send = (c: Command) => f.mock.send(c);

  async function rackOn(kind: "Midi" | "Audio", type: BuiltinDeviceType): Promise<string> {
    const track = await createTrack(f, kind);
    const id = testId();
    await send(cmd("Device", { type: "Insert", id, track, device: { type: "Builtin", device: { type } as BuiltinDevice }, before: null }));
    return id;
  }

  it("loads a factory rack preset's chains, modulators and mappings with seed-derived ids, as one undo step", async () => {
    const rack = await rackOn("Midi", "InstrumentRack");
    const old = testId();
    await send(cmd("Rack", { type: "AddChain", id: old, rack, name: "Old", before: null }));
    const before = project(f);
    const seed = testId();
    const preset = { source: "Factory", id: "instrument-rack/layered-pad" } as const;
    await expect(send(cmd("Preset", { type: "Load", device: rack, preset }))).rejects.toMatchObject({ code: "InvalidArgument" });
    await send(cmd("Preset", { type: "Load", device: rack, preset, seed }));
    const p = project(f);
    expect(p.rack_chains[old]).toBeUndefined();
    const chains = Object.values(p.rack_chains).filter((c) => c.rack === rack);
    expect(chains.map((c) => c.id).sort()).toEqual([deriveId(seed, 0), deriveId(seed, 1)].sort());
    expect(p.rack_chains[deriveId(seed, 0)]?.name).toBe("Body");
    expect(p.devices[deriveId(seed, 2)]).toMatchObject({ name: "Body", chain: deriveId(seed, 0) });
    expect(p.devices[deriveId(seed, 2)]?.params[26]).toBe(1400);
    expect(p.modulators[deriveId(seed, 4)]).toMatchObject({ name: "Breath", device: rack });
    for (let i = 5; i < 9; i++) expect(p.mod_mappings[deriveId(seed, i)]).toBeDefined();
    expect(p.devices[rack]?.params[1]).toBe(0.4);
    await undo(f);
    expect(project(f).rack_chains).toEqual(before.rack_chains);
    expect(project(f).devices).toEqual(before.devices);
    expect(project(f).mod_mappings).toEqual(before.mod_mappings);
  });

  it("saves a rack's structure, and v1 rack presets keep the chains", async () => {
    const rack = await rackOn("Audio", "AudioEffectRack");
    await send(cmd("Preset", { type: "Load", device: rack, preset: { source: "Factory", id: "audio-effect-rack/dub-space" }, seed: testId() }));
    const saved = await send(cmd("Preset", { type: "Save", device: rack, name: "Mine", meta: { tags: [], author: null, description: null }, overwrite: false }));
    if (saved.type !== "Preset") throw new Error(saved.type);
    const snap = rackSnapshot(project(f), rack);
    expect(snap.chains.map((c) => c.name)).toEqual(["Dry", "Echo"]);
    expect(snap.chains[1]!.devices.map((d) => d.name)).toEqual(["Echo", "Tone", "Room"]);
    expect(snap.modulators).toHaveLength(1);
    expect(snap.mappings).toHaveLength(3);

    const other = await rackOn("Audio", "AudioEffectRack");
    await send(cmd("Preset", { type: "Load", device: other, preset: saved.preset.preset, seed: testId() }));
    expect(rackSnapshot(project(f), other)).toEqual(snap);

    const chainsBefore = project(f).rack_chains;
    await send(cmd("Preset", { type: "Load", device: other, preset: { source: "Factory", id: "audio-effect-rack/macros-centered" } }));
    expect(project(f).rack_chains).toEqual(chainsBefore);
    expect(project(f).devices[other]?.params[0]).toBe(0.5);
  });

  it("loads every factory rack preset", async () => {
    for (const [type, kind] of [
      ["InstrumentRack", "Midi"],
      ["AudioEffectRack", "Audio"],
      ["MidiEffectRack", "Midi"],
    ] as const) {
      const rack = await rackOn(kind, type);
      const list = await send(cmd("Preset", { type: "List", device: { type: "Builtin", device: type }, text: null }));
      expect(presetsOf(list).length).toBeGreaterThan(1);
      for (const p of presetsOf(list)) await send(cmd("Preset", { type: "Load", device: rack, preset: p.preset, seed: testId() }));
    }
  });
});
