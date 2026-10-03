/** MockTransport: external devices (v0.3, external-instrument). Pins the contracts-4 stub. */
import { describe, expect, it } from "vitest";
import type { ExternalRouting } from "@/generated";
import { cmd } from "../../cmd";
import { newBuiltinDevice } from "../builtinDevices";
import { createTrack, expectUnsupported, project, testId, useMock } from "./testUtils";

describe("MockTransport external devices (external-instrument)", () => {
  const f = useMock();
  const routing: ExternalRouting = { midi_out: null, midi_channel: 1, audio_send: null, audio_return: null };

  it("inserts both devices as placeholders", async () => {
    const midi = await createTrack(f, "Midi");
    const audio = await createTrack(f, "Audio");
    const inst = testId();
    const fx = testId();
    await f.mock.send(cmd("Device", { type: "Insert", id: inst, track: midi, device: { type: "Builtin", device: newBuiltinDevice("ExternalInstrument") }, before: null }));
    await f.mock.send(cmd("Device", { type: "Insert", id: fx, track: audio, device: { type: "Builtin", device: newBuiltinDevice("ExternalAudioEffect") }, before: null }));
    expect(project(f).devices[inst]?.kind).toEqual({ type: "Builtin", device: { type: "ExternalInstrument", routing } });
    expect(project(f).devices[fx]?.kind).toEqual({ type: "Builtin", device: { type: "ExternalAudioEffect", routing } });
  });

  it("replies Unsupported until the node lands", async () => {
    const midi = await createTrack(f, "Midi");
    const inst = testId();
    await f.mock.send(cmd("Device", { type: "Insert", id: inst, track: midi, device: { type: "Builtin", device: newBuiltinDevice("ExternalInstrument") }, before: null }));
    await expectUnsupported(f, cmd("External", { type: "SetRouting", device: inst, routing: { ...routing, midi_channel: 2 } }));
    await expectUnsupported(f, cmd("External", { type: "ListPorts" }));
    await expectUnsupported(f, cmd("External", { type: "MeasureLatency", device: inst }));
  });
});
