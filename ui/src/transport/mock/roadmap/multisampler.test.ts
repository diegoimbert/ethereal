/** MockTransport: `Device::SetZones` (v0.2, multisampler). */
import { describe, expect, it } from "vitest";
import type { DeviceId, MediaId, MediaSource, SampleZone } from "@/generated";
import { cmd } from "../../cmd";
import { newId } from "../../ids";
import { newBuiltinDevice } from "../builtinDevices";
import { LIBRARY_ID } from "../library";
import { MAX_ZONES } from "./multisampler";
import { type MockFixture, project, trackNamed, undo, useMock } from "./testUtils";

const lib = (path: string): MediaSource => ({ type: "Location", location: { type: "Library", id: LIBRARY_ID }, path });

async function insert(f: MockFixture, type: "MultiSampler" | "Sampler"): Promise<DeviceId> {
  const id: DeviceId = newId();
  const track = trackNamed(f, "Keys").id;
  await f.mock.send(cmd("Device", { type: "Insert", id, track, device: { type: "Builtin", device: newBuiltinDevice(type) }, before: null }));
  return id;
}

async function importLib(f: MockFixture, path: string): Promise<MediaId> {
  const reply = await f.mock.send(cmd("Media", { type: "Import", id: newId(), source: lib(path) }));
  if (reply.type !== "Media") throw new Error(reply.type);
  return reply.media.id;
}

const zone = (media: MediaId | null, lo: number, hi: number): SampleZone => ({
  media,
  root_key: lo,
  tune_cents: 0,
  keys: { lo, hi },
  velocities: { lo: 1, hi: 127 },
  round_robin: 0,
  start: 0,
  end: null,
  looping: false,
  loop_start: 0,
  loop_end: 0,
  loop_crossfade: 0,
  gain: 0,
  pan: 0,
});

const zonesOf = (f: MockFixture, d: DeviceId) => {
  const k = project(f).devices[d]!.kind;
  return k.type === "Builtin" && k.device.type === "MultiSampler" ? k.device.zones : null;
};

const setZones = (f: MockFixture, device: DeviceId, zones: SampleZone[]) => f.mock.send(cmd("Device", { type: "SetZones", device, zones }));

describe("MockTransport SetZones (multisampler)", () => {
  const f = useMock();

  it("replaces the zones in one undo step", async () => {
    const d = await insert(f, "MultiSampler");
    const pad = await importLib(f, "Synths/Pad C.wav");
    const bass = await importLib(f, "Synths/Bass A1.wav");
    const a = [zone(bass, 0, 59), zone(pad, 60, 127)];
    await setZones(f, d, a);
    expect(zonesOf(f, d)).toEqual(a);
    await setZones(f, d, [zone(pad, 0, 127)]);
    await undo(f);
    expect(zonesOf(f, d)).toEqual(a);
    await undo(f);
    expect(zonesOf(f, d)).toEqual([]);
  });

  it("validates like the engine", async () => {
    const d = await insert(f, "MultiSampler");
    const s = await insert(f, "Sampler");
    const pad = await importLib(f, "Synths/Pad C.wav");
    await expect(setZones(f, d, [zone("nope" as MediaId, 0, 127)])).rejects.toMatchObject({ code: "NotFound" });
    await expect(setZones(f, d, [zone(pad, 80, 10)])).rejects.toMatchObject({ code: "InvalidArgument" });
    await expect(setZones(f, d, [{ ...zone(pad, 0, 127), tune_cents: 150 }])).rejects.toMatchObject({ code: "InvalidArgument" });
    await expect(setZones(f, d, Array.from({ length: MAX_ZONES + 1 }, () => zone(null, 0, 127)))).rejects.toMatchObject({
      code: "InvalidArgument",
    });
    await expect(setZones(f, s, [])).rejects.toMatchObject({ code: "InvalidArgument" });
    expect(zonesOf(f, d)).toEqual([]);
    // Empty zones need no media.
    await setZones(f, d, [zone(null, 0, 127)]);
    expect(zonesOf(f, d)).toHaveLength(1);
  });
});
