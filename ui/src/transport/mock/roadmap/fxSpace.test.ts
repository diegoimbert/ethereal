/** MockTransport: convolution reverb IRs (v0.3, fx-space): `Device::{SetIr, ListFactoryIrs}`. */
import { describe, expect, it } from "vitest";
import type { DeviceId, IrSource, MediaId, MediaSource } from "@/generated";
import { cmd } from "../../cmd";
import { newBuiltinDevice } from "../builtinDevices";
import { LIBRARY_ID } from "../library";
import { type MockFixture, createTrack, project, testId, undo, useMock } from "./testUtils";

async function insertReverb(f: MockFixture): Promise<DeviceId> {
  const track = await createTrack(f, "Audio");
  const id = testId();
  await f.mock.send(cmd("Device", { type: "Insert", id, track, device: { type: "Builtin", device: newBuiltinDevice("ConvolutionReverb") }, before: null }));
  return id;
}

const irOf = (f: MockFixture, d: DeviceId) => {
  const k = project(f).devices[d]!.kind;
  return k.type === "Builtin" && k.device.type === "ConvolutionReverb" ? k.device.ir : undefined;
};

const setIr = (f: MockFixture, device: DeviceId, ir: IrSource | null) => f.mock.send(cmd("Device", { type: "SetIr", device, ir }));

describe("MockTransport convolution reverb (fx-space)", () => {
  const f = useMock();

  it("inserts with no IR and the generated descriptor", async () => {
    const id = await insertReverb(f);
    expect(project(f).devices[id]?.kind).toEqual({ type: "Builtin", device: { type: "ConvolutionReverb", ir: null } });
    const reply = await f.mock.send(cmd("Device", { type: "GetDescriptor", device: id }));
    expect(reply).toMatchObject({ type: "Descriptor", descriptor: { name: "Convolution Reverb" } });
  });

  it("lists the factory IRs like the engine", async () => {
    const reply = await f.mock.send(cmd("Device", { type: "ListFactoryIrs" }));
    if (reply.type !== "FactoryIrs") throw new Error(reply.type);
    expect(reply.irs.map((i) => i.id).slice(0, 4)).toEqual(["room", "chamber", "plate", "hall"]);
    expect(reply.irs.find((i) => i.id === "hall")).toMatchObject({ name: "Concert Hall", category: "Hall", channels: 2 });
  });

  it("sets the IR in one undo step per change", async () => {
    const d = await insertReverb(f);
    await setIr(f, d, { type: "Factory", id: "hall" });
    expect(irOf(f, d)).toEqual({ type: "Factory", id: "hall" });
    const src: MediaSource = { type: "Location", location: { type: "Library", id: LIBRARY_ID }, path: "Drums/Kick.wav" };
    const reply = await f.mock.send(cmd("Media", { type: "Import", id: testId(), source: src }));
    if (reply.type !== "Media") throw new Error(reply.type);
    const media: MediaId = reply.media.id;
    await setIr(f, d, { type: "Media", media });
    expect(irOf(f, d)).toEqual({ type: "Media", media });
    await setIr(f, d, null);
    expect(irOf(f, d)).toBeNull();
    await undo(f);
    expect(irOf(f, d)).toEqual({ type: "Media", media });
    await undo(f);
    expect(irOf(f, d)).toEqual({ type: "Factory", id: "hall" });
  });

  it("validates the source and the device", async () => {
    const d = await insertReverb(f);
    const before = project(f);
    await expect(setIr(f, d, { type: "Factory", id: "nope" })).rejects.toMatchObject({ code: "NotFound" });
    await expect(setIr(f, d, { type: "Media", media: "01K00000000000000000000000" })).rejects.toMatchObject({ code: "NotFound" });
    await expect(setIr(f, testId(), { type: "Factory", id: "hall" })).rejects.toMatchObject({ code: "NotFound" });
    expect(project(f)).toEqual(before);
    const track = project(f).devices[d]!.track;
    const comp = testId();
    await f.mock.send(cmd("Device", { type: "Insert", id: comp, track, device: { type: "Builtin", device: newBuiltinDevice("Compressor") }, before: null }));
    await expect(setIr(f, comp, { type: "Factory", id: "hall" })).rejects.toMatchObject({ code: "InvalidArgument" });
  });
});
