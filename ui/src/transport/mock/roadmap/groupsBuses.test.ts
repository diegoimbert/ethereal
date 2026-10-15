/** MockTransport: `Track::{GroupSelected, Ungroup, SetVca}` (v0.2, groups-buses). */
import { describe, expect, it } from "vitest";
import type { Project, TrackId } from "@/generated";
import { compareOrderKeys } from "@/state/orderKey";
import { cmd } from "../../cmd";
import { newId } from "../../ids";
import { newBuiltinDevice } from "../builtinDevices";
import { type MockFixture, project, trackNamed, undo, useMock } from "./testUtils";

const children = (p: Project, parent: TrackId | null) =>
  Object.values(p.tracks)
    .filter((t) => t.parent === parent)
    .sort((a, b) => compareOrderKeys(a.order, b.order))
    .map((t) => t.name);

const group = (f: MockFixture, ids: TrackId[], name: string | null = null) => {
  const id = newId();
  return f.mock.send(cmd("Track", { type: "GroupSelected", ids, group: id, name })).then(() => id);
};

describe("MockTransport Track (groups-buses)", () => {
  const f = useMock();

  it("groups siblings in place (one undo step) and ungroups them back", async () => {
    const keys = trackNamed(f, "Keys");
    const bass = trackNamed(f, "Bass");
    const before = children(project(f), null);
    // Selection order does not matter: children keep the track order.
    const g = await group(f, [bass.id, keys.id]);
    const top = children(project(f), null);
    expect(top.indexOf("Group")).toBe(before.indexOf("Keys"));
    expect(children(project(f), g)).toEqual(["Keys", "Bass"]);
    expect(project(f).tracks[g]!.color).toBe(keys.color);
    await f.mock.send(cmd("Track", { type: "Ungroup", group: g, force: false }));
    expect(children(project(f), null)).toEqual(before);
    await undo(f);
    expect(project(f).tracks[g]).toBeDefined();
    await undo(f);
    expect(project(f).tracks[g]).toBeUndefined();
    expect(children(project(f), null)).toEqual(before);
  });

  it("nests, rejects mixed parents and special tracks", async () => {
    const keys = trackNamed(f, "Keys");
    const bass = trackNamed(f, "Bass");
    const outer = await group(f, [keys.id, bass.id], "Band");
    const inner = await group(f, [bass.id], "Low end");
    expect(project(f).tracks[inner]!.parent).toBe(outer);
    expect(children(project(f), outer)).toEqual(["Keys", "Low end"]);
    await expect(group(f, [keys.id, trackNamed(f, "Drums").id])).rejects.toMatchObject({ code: "InvalidArgument" });
    await expect(group(f, [trackNamed(f, "A Delay").id])).rejects.toMatchObject({ code: "InvalidArgument" });
    await expect(group(f, [])).rejects.toMatchObject({ code: "InvalidArgument" });
  });

  it("ungroup needs force when the group has devices", async () => {
    const g = await group(f, [trackNamed(f, "Keys").id]);
    await f.mock.send(cmd("Device", { type: "Insert", id: newId(), track: g, device: { type: "Builtin", device: newBuiltinDevice("Delay") }, before: null }));
    await expect(f.mock.send(cmd("Track", { type: "Ungroup", group: g, force: false }))).rejects.toMatchObject({ code: "InvalidState" });
    await f.mock.send(cmd("Track", { type: "Ungroup", group: g, force: true }));
    expect(project(f).tracks[g]).toBeUndefined();
    expect(Object.values(project(f).devices).some((d) => d.track === g)).toBe(false);
  });

  it("assigns VCAs, rejects cycles and unassigns on delete", async () => {
    const keys = trackNamed(f, "Keys");
    const v1 = newId();
    const v2 = newId();
    for (const id of [v1, v2]) {
      await f.mock.send(cmd("Track", { type: "Create", id, kind: "Vca", name: null, color: null, parent: null, before: null }));
    }
    await f.mock.send(cmd("Track", { type: "SetVca", id: keys.id, vca: v1 }));
    await f.mock.send(cmd("Track", { type: "SetVca", id: v1, vca: v2 }));
    expect(project(f).tracks[keys.id]!.vca).toBe(v1);
    await expect(f.mock.send(cmd("Track", { type: "SetVca", id: v2, vca: v1 }))).rejects.toMatchObject({ code: "InvalidArgument" });
    await expect(f.mock.send(cmd("Track", { type: "SetVca", id: v2, vca: keys.id }))).rejects.toMatchObject({ code: "InvalidArgument" });
    await f.mock.send(cmd("Track", { type: "Delete", id: v1 }));
    expect(project(f).tracks[keys.id]!.vca).toBeUndefined();
    expect(project(f).tracks[v2]!.vca).toBeUndefined();
  });
});
