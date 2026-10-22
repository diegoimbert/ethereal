/** Modulation reach (mirrors `ether_model::modulation`): sources reach inside nested racks. */
import { describe, expect, it } from "vitest";
import type { Device, DrumPad, Project, RackChain } from "@/generated";
import { canTarget, isInsideRack, macroIndex, macroParam } from "./model";

const rack = (id: string, chain?: string): Device =>
  ({
    id,
    track: "t",
    order: "a",
    name: id,
    enabled: true,
    kind: { type: "Builtin", device: { type: "AudioEffectRack" } },
    params: {},
    sidechain: null,
    pad: null,
    ...(chain ? { chain } : {}),
  }) as Device;
const fx = (id: string, chain: string): Device =>
  ({
    id,
    track: "t",
    order: "a",
    name: id,
    enabled: true,
    kind: { type: "Builtin", device: { type: "Reverb" } },
    params: {},
    sidechain: null,
    pad: null,
    chain,
  }) as Device;
const chain = (id: string, of: string): RackChain => ({ id, rack: of }) as RackChain;

// outer ─ c1 ─ inner ─ c2 ─ reverb; other (a separate rack).
const project = {
  devices: { outer: rack("outer"), inner: rack("inner", "c1"), reverb: fx("reverb", "c2"), other: rack("other") },
  rack_chains: { c1: chain("c1", "outer"), c2: chain("c2", "inner") },
  modulators: {},
  mod_mappings: {},
} as unknown as Project;

describe("modulation reach", () => {
  it("a device is inside every rack above it", () => {
    expect(isInsideRack(project, project.devices.reverb!, "inner")).toBe(true);
    expect(isInsideRack(project, project.devices.reverb!, "outer")).toBe(true);
    expect(isInsideRack(project, project.devices.reverb!, "other")).toBe(false);
    expect(isInsideRack(project, project.devices.outer!, "outer")).toBe(false);
  });

  it("macros reach devices at any depth inside their rack, not outside it", () => {
    const reverb = project.devices.reverb!;
    expect(canTarget(project, { type: "Macro", rack: "outer", index: 0 }, reverb, 0)).toBe(true);
    expect(canTarget(project, { type: "Macro", rack: "inner", index: 0 }, reverb, 0)).toBe(true);
    expect(canTarget(project, { type: "Macro", rack: "other", index: 0 }, reverb, 0)).toBe(false);
    // The outer macro reaches the inner rack's params, the inner macro not the outer rack.
    expect(canTarget(project, { type: "Macro", rack: "outer", index: 0 }, project.devices.inner!, 0)).toBe(true);
    expect(canTarget(project, { type: "Macro", rack: "inner", index: 0 }, project.devices.outer!, 9)).toBe(false);
  });
});

// inst ─ c1 ─ drums ─ p1 ─ [kick, padfx ─ c3 ─ chorus]; top (another drum rack).
const dev = (id: string, type: string, at: { chain?: string; pad?: string } = {}): Device =>
  ({
    id,
    track: "t",
    order: "a",
    name: id,
    enabled: true,
    kind: { type: "Builtin", device: { type } },
    params: {},
    sidechain: null,
    pad: at.pad ?? null,
    ...(at.chain ? { chain: at.chain } : {}),
  }) as Device;
const drumProject = {
  devices: {
    inst: dev("inst", "InstrumentRack"),
    drums: dev("drums", "DrumRack", { chain: "c1" }),
    kick: dev("kick", "Synth", { pad: "p1" }),
    padfx: dev("padfx", "AudioEffectRack", { pad: "p1" }),
    chorus: dev("chorus", "Chorus", { chain: "c3" }),
    top: dev("top", "DrumRack"),
  },
  rack_chains: { c1: chain("c1", "inst"), c3: chain("c3", "padfx") },
  drum_pads: { p1: { id: "p1", rack: "drums" } as DrumPad },
  modulators: {},
  mod_mappings: {},
} as unknown as Project;

describe("drum rack macros", () => {
  const d = drumProject.devices;
  it("are the drum rack's params 2..18, a chain rack's 0..8 then 9..17 (after its selector)", () => {
    expect(macroParam(d.drums, 0)).toBe(2);
    expect(macroParam(d.drums, 7)).toBe(9);
    expect(macroParam(d.drums, 8)).toBe(10);
    expect(macroParam(d.drums, 15)).toBe(17);
    expect(macroParam(d.drums, 16)).toBe(null);
    expect(macroIndex(d.drums, 1)).toBe(null);
    expect(macroIndex(d.drums, 5)).toBe(3);
    expect(macroIndex(d.drums, 17)).toBe(15);
    expect(macroParam(d.inst, 0)).toBe(0);
    expect(macroParam(d.inst, 7)).toBe(7);
    expect(macroParam(d.inst, 8)).toBe(9);
    expect(macroParam(d.inst, 15)).toBe(16);
    expect(macroIndex(d.inst, 8)).toBe(null);
    expect(macroIndex(d.inst, 9)).toBe(8);
    expect(macroParam(d.kick, 0)).toBe(null);
  });

  it("reach the pads' devices at any depth, and outer racks' macros reach into the pads", () => {
    expect(isInsideRack(drumProject, d.chorus!, "drums")).toBe(true);
    expect(isInsideRack(drumProject, d.kick!, "inst")).toBe(true);
    const drumMacro = { type: "Macro", rack: "drums", index: 0 } as const;
    expect(canTarget(drumProject, drumMacro, d.kick!, 0)).toBe(true);
    expect(canTarget(drumProject, drumMacro, d.chorus!, 0)).toBe(true);
    expect(canTarget(drumProject, { type: "Macro", rack: "inst", index: 0 }, d.chorus!, 0)).toBe(true);
    expect(canTarget(drumProject, { type: "Macro", rack: "top", index: 0 }, d.kick!, 0)).toBe(false);
    // The rack's own volume and pan, never its macros.
    expect(canTarget(drumProject, drumMacro, d.drums!, 0)).toBe(true);
    expect(canTarget(drumProject, drumMacro, d.drums!, 2)).toBe(false);
  });
});
