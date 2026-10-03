// midi-fx: where a new device goes in a chain (CONTRACTS.md §12.4.4: MIDI effects precede the
// instrument), so inserts from the picker and the Devices pane are never refused.
import { describe, expect, it } from "vitest";
import type { BuiltinDeviceType, Device } from "@/generated";
import { chainInsertBefore } from "./deviceInsert";

const dev = (id: string, type: BuiltinDeviceType): Device =>
  ({ id, kind: { type: "Builtin", device: { type } } }) as unknown as Device;

const plugin = (id: string): Device =>
  ({ id, kind: { type: "Plugin", plugin: { plugin_id: "com.test.fx" } } }) as unknown as Device;

describe("chainInsertBefore", () => {
  const chain = [dev("arp", "Arpeggiator"), dev("chord", "Chord"), dev("synth", "Synth"), dev("delay", "Delay")];

  it("puts MIDI effects before the instrument", () => {
    expect(chainInsertBefore(chain, "NoteEffect")).toBe("synth");
    expect(chainInsertBefore([dev("synth", "Synth")], "NoteEffect")).toBe("synth");
  });

  it("appends MIDI effects to a chain of MIDI effects (or an empty one)", () => {
    expect(chainInsertBefore([dev("arp", "Arpeggiator")], "NoteEffect")).toBeNull();
    expect(chainInsertBefore([], "NoteEffect")).toBeNull();
  });

  it("puts an instrument after the MIDI effects", () => {
    expect(chainInsertBefore([dev("arp", "Arpeggiator"), dev("eq", "Eq")], "Instrument")).toBe("eq");
    expect(chainInsertBefore([dev("vel", "Velocity")], "Instrument")).toBeNull();
    expect(chainInsertBefore([dev("eq", "Eq")], "Instrument")).toBe("eq");
  });

  it("appends audio effects", () => {
    expect(chainInsertBefore(chain, "AudioEffect")).toBeNull();
  });

  it("treats MIDI effect racks as MIDI effects and plugins as not", () => {
    expect(chainInsertBefore([dev("rack", "MidiEffectRack"), dev("synth", "Synth")], "NoteEffect")).toBe("synth");
    expect(chainInsertBefore([plugin("p")], "NoteEffect")).toBe("p");
  });
});
