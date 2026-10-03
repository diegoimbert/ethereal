/** MockTransport: convolution reverb (v0.3, fx-space). Pins the contracts-4 placeholder. */
import { describe, expect, it } from "vitest";
import { cmd } from "../../cmd";
import { newBuiltinDevice } from "../builtinDevices";
import { createTrack, expectUnsupported, project, testId, useMock } from "./testUtils";

describe("MockTransport convolution reverb (fx-space)", () => {
  const f = useMock();

  it("inserts as a placeholder with its generated descriptor", async () => {
    const track = await createTrack(f, "Audio");
    const id = testId();
    await f.mock.send(cmd("Device", { type: "Insert", id, track, device: { type: "Builtin", device: newBuiltinDevice("ConvolutionReverb") }, before: null }));
    expect(project(f).devices[id]?.kind).toEqual({ type: "Builtin", device: { type: "ConvolutionReverb", ir: null } });
    const reply = await f.mock.send(cmd("Device", { type: "GetDescriptor", device: id }));
    expect(reply).toMatchObject({ type: "Descriptor", descriptor: { name: "Convolution Reverb" } });
  });

  it("replies Unsupported for IRs until the node lands", async () => {
    const track = await createTrack(f, "Audio");
    const id = testId();
    await f.mock.send(cmd("Device", { type: "Insert", id, track, device: { type: "Builtin", device: newBuiltinDevice("ConvolutionReverb") }, before: null }));
    await expectUnsupported(f, cmd("Device", { type: "SetIr", device: id, ir: { type: "Factory", id: "hall" } }));
    await expectUnsupported(f, cmd("Device", { type: "ListFactoryIrs" }));
  });
});
