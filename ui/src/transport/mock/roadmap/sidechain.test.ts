/** MockTransport: sidechain routing (sidechain). */
import { describe, expect, it } from "vitest";
import type { BuiltinDeviceType, TrackId } from "@/generated";
import { cmd } from "../../cmd";
import { newBuiltinDevice } from "../builtinDevices";
import { newId } from "../../ids";
import { type MockFixture, project, trackNamed, undo, useMock } from "./testUtils";

async function insert(f: MockFixture, track: TrackId, type: BuiltinDeviceType): Promise<string> {
  const id = newId();
  await f.mock.send(cmd("Device", { type: "Insert", id, track, device: { type: "Builtin", device: newBuiltinDevice(type) }, before: null }));
  return id;
}

const setSidechain = (f: MockFixture, device: string, source: TrackId | null) =>
  f.mock.send(cmd("Device", { type: "SetSidechain", device, source }));

describe("MockTransport sidechain", () => {
  const f = useMock();

  it("sets a source, rejects the own track, and is cut when the source track is deleted", async () => {
    const drums = trackNamed(f, "Drums");
    const keys = trackNamed(f, "Keys");
    const comp = await insert(f, keys.id, "Compressor");
    await expect(setSidechain(f, comp, keys.id)).rejects.toMatchObject({ code: "InvalidArgument" });
    await setSidechain(f, comp, drums.id);
    expect(project(f).devices[comp]!.sidechain).toBe(drums.id);
    await f.mock.send(cmd("Track", { type: "Delete", id: drums.id }));
    expect(project(f).devices[comp]!.sidechain).toBeNull();
    await undo(f);
    expect(project(f).devices[comp]!.sidechain).toBe(drums.id);
  });

  it("is one undo step and clearing restores nothing else", async () => {
    const drums = trackNamed(f, "Drums");
    const keys = trackNamed(f, "Keys");
    const lim = await insert(f, keys.id, "Limiter");
    await setSidechain(f, lim, drums.id);
    await setSidechain(f, lim, null);
    expect(project(f).devices[lim]!.sidechain).toBeNull();
    await undo(f);
    expect(project(f).devices[lim]!.sidechain).toBe(drums.id);
    await undo(f);
    expect(project(f).devices[lim]!.sidechain).toBeNull();
  });

  it("rejects master sources, devices without a sidechain input and cycles", async () => {
    const drums = trackNamed(f, "Drums");
    const keys = trackNamed(f, "Keys");
    const master = Object.values(project(f).tracks).find((t) => t.kind === "Master")!;
    const comp = await insert(f, keys.id, "Compressor");
    const delay = await insert(f, keys.id, "Delay");
    await expect(setSidechain(f, comp, master.id)).rejects.toMatchObject({ code: "InvalidArgument" });
    await expect(setSidechain(f, delay, drums.id)).rejects.toMatchObject({ code: "InvalidArgument" });
    await expect(setSidechain(f, comp, "nope" as TrackId)).rejects.toMatchObject({ code: "NotFound" });
    // Keys listens to Drums; Drums listening to Keys would be a cycle.
    await setSidechain(f, comp, drums.id);
    const back = await insert(f, drums.id, "Compressor");
    await expect(setSidechain(f, back, keys.id)).rejects.toMatchObject({ code: "InvalidArgument" });
    expect(project(f).devices[back]!.sidechain).toBeNull();
  });
});
