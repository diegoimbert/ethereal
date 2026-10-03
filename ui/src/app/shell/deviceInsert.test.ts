import { describe, expect, it } from "vitest";
import type { DeviceDescriptor, Project, Track } from "@/generated";
import { devicesOfTrack, tracksOrdered } from "@/state";
import { fetchBuiltinTypes } from "@/features/devices/descriptors";
import { MockTransport } from "@/transport";
import { insertDeviceCommand } from "./deviceInsert";

async function setup() {
  const mock = new MockTransport({ timers: "manual", seed: 7 });
  const project: Project = await mock.connect();
  const types = await fetchBuiltinTypes(mock);
  const type = (name: string): DeviceDescriptor => types.find((d) => d.device_type.type === "Builtin" && d.device_type.device === name)!;
  const track = (kind: Track["kind"]) => tracksOrdered(project).find((t) => t.kind === kind)!;
  return { mock, project, type, track };
}

describe("insertDeviceCommand", () => {
  it("an instrument replaces the track's instrument in place, as one step", async () => {
    const { mock, project, type, track } = await setup();
    const keys = track("Midi");
    const synth = devicesOfTrack(project, keys.id)[0]!.id;
    expect(await insertDeviceCommand(mock, project, keys, type("Sampler"))).toMatchObject({
      domain: "Edit",
      command: {
        type: "Batch",
        label: "Replace Instrument",
        commands: [
          { domain: "Device", command: { type: "Insert", track: keys.id, before: synth } },
          { domain: "Device", command: { type: "Remove", id: synth } },
        ],
      },
    });
  });

  it("effects go last; instruments never go on audio tracks", async () => {
    const { mock, project, type, track } = await setup();
    const audio = track("Audio");
    expect(await insertDeviceCommand(mock, project, audio, type("Delay"))).toMatchObject({
      domain: "Device",
      command: { type: "Insert", track: audio.id, before: null },
    });
    expect(await insertDeviceCommand(mock, project, audio, type("Sampler"))).toBeNull();
  });
});
