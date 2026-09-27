/** MockTransport: markers and v2 clip commands (clip-editing). */
import { describe, expect, it } from "vitest";
import { cmd } from "../../cmd";
import { newId } from "../../ids";
import { project, useMock } from "./testUtils";

describe("MockTransport clipEditing", () => {
  const f = useMock();

  it("markers: add (idempotent), move, rename, remove", async () => {
    const id = newId();
    const add = cmd("Marker", { type: "Add", id, position: 8, name: null, color: null });
    await f.mock.send(add);
    await f.mock.send(add);
    expect(Object.keys(project(f).markers)).toEqual([id]);
    expect(project(f).markers[id]!.name).toBe("Marker 1");
    await f.mock.send(cmd("Marker", { type: "Move", id, position: 12 }));
    await f.mock.send(cmd("Marker", { type: "Rename", id, name: "Chorus" }));
    expect(project(f).markers[id]).toMatchObject({ position: 12, name: "Chorus" });
    await f.mock.send(cmd("Marker", { type: "Remove", ids: [id] }));
    expect(project(f).markers[id]).toBeUndefined();
  });
});
