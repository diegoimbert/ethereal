/** MockTransport: external devices (v0.3, external-instrument), mirroring the controller. */
import { describe, expect, it } from "vitest";
import type { ExternalRouting } from "@/generated";
import { cmd } from "../../cmd";
import { newBuiltinDevice } from "../builtinDevices";
import { createTrack, expectUnsupported, project, testId, useMock } from "./testUtils";
import { MOCK_PORTS } from "./external";

describe("MockTransport external devices (external-instrument)", () => {
  const f = useMock();
  const routing: ExternalRouting = { midi_out: null, midi_channel: 1, audio_send: null, audio_return: null };

  async function devices() {
    const midi = await createTrack(f, "Midi");
    const audio = await createTrack(f, "Audio");
    const inst = testId();
    const fx = testId();
    await f.mock.send(cmd("Device", { type: "Insert", id: inst, track: midi, device: { type: "Builtin", device: newBuiltinDevice("ExternalInstrument") }, before: null }));
    await f.mock.send(cmd("Device", { type: "Insert", id: fx, track: audio, device: { type: "Builtin", device: newBuiltinDevice("ExternalAudioEffect") }, before: null }));
    return { inst, fx };
  }

  it("inserts both devices with an empty routing", async () => {
    const { inst, fx } = await devices();
    expect(project(f).devices[inst]?.kind).toEqual({ type: "Builtin", device: { type: "ExternalInstrument", routing } });
    expect(project(f).devices[fx]?.kind).toEqual({ type: "Builtin", device: { type: "ExternalAudioEffect", routing } });
  });

  it("sets the routing as one undo step and validates it", async () => {
    const { inst, fx } = await devices();
    const r: ExternalRouting = { midi_out: "Mock Synth", midi_channel: 4, audio_send: null, audio_return: { first: 2, count: 1 } };
    await f.mock.send(cmd("External", { type: "SetRouting", device: inst, routing: r }));
    expect(project(f).devices[inst]?.kind).toEqual({ type: "Builtin", device: { type: "ExternalInstrument", routing: r } });
    const bad: [string, ExternalRouting][] = [
      [inst, { ...r, audio_send: { first: 0, count: 2 } }],
      [fx, { ...routing, midi_out: "Mock Synth" }],
      [inst, { ...r, midi_channel: 0 }],
      [fx, { ...routing, audio_return: { first: 0, count: 3 } }],
    ];
    for (const [device, routing] of bad) {
      await expect(f.mock.send(cmd("External", { type: "SetRouting", device, routing }))).rejects.toMatchObject({ code: "InvalidArgument" });
    }
    await f.mock.send(cmd("Edit", { type: "Undo" }));
    expect(project(f).devices[inst]?.kind).toEqual({ type: "Builtin", device: { type: "ExternalInstrument", routing } });
  });

  it("lists demo ports and cannot measure without hardware", async () => {
    const { fx } = await devices();
    expect(await f.mock.send(cmd("External", { type: "ListPorts" }))).toEqual({ type: "HardwarePorts", ports: MOCK_PORTS });
    await expectUnsupported(f, cmd("External", { type: "MeasureLatency", device: fx }));
  });
});
