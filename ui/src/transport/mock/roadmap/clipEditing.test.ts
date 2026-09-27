/** MockTransport: markers and v2 clip commands (clip-editing). */
import { describe, expect, it } from "vitest";
import { cmd } from "../../cmd";
import { newId } from "../../ids";
import { isCrossfade } from "./clipEditing";
import { project, trackNamed, undo, useMock } from "./testUtils";

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

  const drumClip = () => Object.values(project(f).clips).find((c) => c.track === trackNamed(f, "Drums").id)!;

  it("fade curves and reverse (audio only), one undo step each", async () => {
    const id = drumClip().id;
    await f.mock.send(cmd("Clip", { type: "SetFadeCurves", id, fade_in: { type: "EqualPower" }, fade_out: null }));
    await f.mock.send(cmd("Clip", { type: "SetReversed", id, reversed: true }));
    expect(project(f).clips[id]!.content).toMatchObject({ fade_in_curve: { type: "EqualPower" }, fade_out_curve: { type: "Linear" }, reversed: true });
    await expect(f.mock.send(cmd("Clip", { type: "SetFadeCurves", id, fade_in: { type: "Curve", tension: 2 }, fade_out: null }))).rejects.toThrow();
    await undo(f);
    expect(project(f).clips[id]!.content).toMatchObject({ reversed: false, fade_in_curve: { type: "EqualPower" } });
    await undo(f);
    expect(project(f).clips[id]!.content).toMatchObject({ fade_in_curve: { type: "Linear" } });
  });

  it("crossfade extends both clips around the boundary (same as Rust)", async () => {
    const a = drumClip();
    const bId = newId();
    await f.mock.send(cmd("Clip", { type: "Split", id: a.id, at: 8, new_id: bId }));
    await f.mock.send(cmd("Clip", { type: "Crossfade", first: a.id, second: bId, length: 1, curve: { type: "EqualPower" } }));
    const [ca, cb] = [project(f).clips[a.id]!, project(f).clips[bId]!];
    expect([ca.start, ca.length, ca.offset]).toEqual([0, 8.5, 0]);
    expect([cb.start, cb.length, cb.offset]).toEqual([7.5, 8.5, 7.5]);
    expect(ca.content).toMatchObject({ fade_out: 1, fade_out_curve: { type: "EqualPower" } });
    expect(cb.content).toMatchObject({ fade_in: 1, fade_in_curve: { type: "EqualPower" } });
    expect(isCrossfade(ca, cb)).toBe(true);
    await undo(f);
    expect(project(f).clips[a.id]!.length).toBe(8);
    expect(project(f).clips[bId]!.start).toBe(8);
    await expect(
      f.mock.send(cmd("Clip", { type: "Crossfade", first: bId, second: a.id, length: 1, curve: { type: "Linear" } })),
    ).rejects.toThrow();
  });
});
