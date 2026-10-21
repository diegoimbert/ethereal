// Grouping devices into a rack: which ones can be (next to each other on one track's chain)
// and the rack type that holds them.
import { describe, expect, it } from "vitest";
import type { Device, Project } from "@/generated";
import { groupable, rackTypeForAll } from "./group";

const dev = (id: string, order: string, extra: Partial<Device> = {}) => ({ id, order, track: "t", pad: null, chain: null, ...extra }) as unknown as Device;
const project = (...ds: Device[]) => ({ devices: Object.fromEntries(ds.map((d) => [d.id, d])) }) as unknown as Project;

describe("group into rack", () => {
  const p = project(dev("a", "1"), dev("b", "2"), dev("c", "3"), dev("x", "1", { track: "u" }), dev("r", "4", { chain: "ch" }));

  it("takes devices next to each other on one track chain, in chain order", () => {
    expect(groupable(p, ["c", "b"])?.map((d) => d.id)).toEqual(["b", "c"]);
    expect(groupable(p, ["a", "c"])).toBeNull();
    expect(groupable(p, ["a", "x"])).toBeNull();
    expect(groupable(p, ["r"])).toBeNull();
    expect(groupable(p, [])).toBeNull();
  });

  it("picks the rack that holds them all", () => {
    expect(rackTypeForAll(["AudioEffect", "AudioEffect"])).toBe("AudioEffectRack");
    expect(rackTypeForAll(["NoteEffect"])).toBe("MidiEffectRack");
    expect(rackTypeForAll(["NoteEffect", "Instrument", "AudioEffect"])).toBe("InstrumentRack");
    expect(rackTypeForAll(["Instrument"])).toBe("InstrumentRack");
  });
});
