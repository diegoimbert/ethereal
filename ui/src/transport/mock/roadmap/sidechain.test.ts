/** MockTransport: sidechain routing (sidechain). */
import { describe, expect, it } from "vitest";
import { cmd } from "../../cmd";
import { newId } from "../../ids";
import { project, trackNamed, undo, useMock } from "./testUtils";

describe("MockTransport sidechain", () => {
  const f = useMock();

  it("sets a source, rejects the own track, and is cut when the source track is deleted", async () => {
    const drums = trackNamed(f, "Drums");
    const keys = trackNamed(f, "Keys");
    const comp = newId();
    await f.mock.send(cmd("Device", { type: "Insert", id: comp, track: keys.id, device: { type: "Builtin", device: { type: "Compressor" } }, before: null }));
    await expect(f.mock.send(cmd("Device", { type: "SetSidechain", device: comp, source: keys.id }))).rejects.toMatchObject({
      code: "InvalidArgument",
    });
    await f.mock.send(cmd("Device", { type: "SetSidechain", device: comp, source: drums.id }));
    expect(project(f).devices[comp]!.sidechain).toBe(drums.id);
    await f.mock.send(cmd("Track", { type: "Delete", id: drums.id }));
    expect(project(f).devices[comp]!.sidechain).toBeNull();
    await undo(f);
    expect(project(f).devices[comp]!.sidechain).toBe(drums.id);
  });
});
