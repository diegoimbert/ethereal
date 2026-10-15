/** Modulation reach (mirrors `ether_model::modulation`): sources reach inside nested racks. */
import { describe, expect, it } from "vitest";
import type { Device, Project, RackChain } from "@/generated";
import { canTarget, isInsideRack } from "./model";

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
