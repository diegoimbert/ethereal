// midi-fx: where a new device goes in a chain (CONTRACTS.md §12.4.4: MIDI effects precede the
// instrument), so inserts from the picker and the Devices pane are never refused.
import { describe, expect, it } from "vitest";
import type { BuiltinDeviceType, Device, DeviceDescriptor, Project } from "@/generated";
import { devicesOfTrack, tracksOrdered } from "@/state";
import { fetchBuiltinTypes } from "@/features/devices/descriptors";
import { MockTransport } from "@/transport";
import { chainInsertBefore, insertDeviceCommand } from "./deviceInsert";

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

describe("insertDeviceCommand (MIDI effects)", () => {
  it("puts a MIDI effect before the track's instrument, and an instrument after it replaces the instrument", async () => {
    const mock = new MockTransport({ timers: "manual", seed: 7 });
    const project: Project = await mock.connect();
    const types = await fetchBuiltinTypes(mock);
    const type = (name: string): DeviceDescriptor => types.find((d) => d.device_type.type === "Builtin" && d.device_type.device === name)!;
    const keys = tracksOrdered(project).find((t) => t.kind === "Midi")!;
    const synth = devicesOfTrack(project, keys.id)[0]!.id;
    expect(await insertDeviceCommand(mock, project, keys, type("Arpeggiator"))).toMatchObject({
      domain: "Device",
      command: { type: "Insert", track: keys.id, before: synth },
    });
  });
});
