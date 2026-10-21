// The selector's rules for a device keying another chain of the rack it is in (mirrors
// `ether_model`: that chain runs first, before its volume, never in a loop).
import { describe, expect, it } from "vitest";
import type { Device, Project, SidechainTap } from "@/generated";
import { tapError } from "./routing";

const dev = (id: string, order: string, chain: string | null, extra: Partial<Device> = {}) =>
  ({ id, order, track: "t", pad: null, chain, sidechain: null, name: id, ...extra }) as unknown as Device;

function project(devices: Device[]) {
  return {
    tracks: { t: { id: "t", kind: "Audio", output: { type: "Default" }, parent: null } },
    sends: {},
    drum_pads: {},
    rack_chains: {
      a: { id: "a", rack: "rack", order: "a", name: "a" },
      b: { id: "b", rack: "rack", order: "b", name: "b" },
    },
    devices: Object.fromEntries(devices.map((d) => [d.id, d])),
  } as unknown as Project;
}

const on = (chain: string, point: SidechainTap["point"]): SidechainTap => ({ point, chain: { type: "Rack", chain } });

describe("a chain keying another chain of its rack", () => {
  const rack = dev("rack", "m", null);
  const comp = dev("comp", "m", "a");
  const gate = dev("gate", "m", "b");

  it("is allowed before the chain's volume (pre or post FX), never its own chain", () => {
    const p = project([rack, comp, gate]);
    expect(tapError(p, comp, "t", on("b", "PostFx"))).toBeNull();
    expect(tapError(p, comp, "t", on("b", "PreFx"))).toBeNull();
    expect(tapError(p, comp, "t", on("b", "PostFader"))).toBe("sibling_post_fader");
    expect(tapError(p, comp, "t", on("a", "PostFx"))).toBe("holds_device");
  });

  it("never both ways (a loop)", () => {
    const keyed = { ...comp, sidechain: "t", sidechain_tap: on("b", "PostFx") } as Device;
    const p = project([rack, keyed, gate]);
    expect(tapError(p, gate, "t", on("a", "PostFx"))).toBe("loop");
    // Its own new choice replaces its old one: no loop with itself.
    expect(tapError(p, keyed, "t", on("b", "PreFx"))).toBeNull();
  });
});
