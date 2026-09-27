/** MockTransport: groove (groove). */
import { describe, expect, it } from "vitest";
import { cmd } from "../../cmd";
import { swingOffset } from "./groove";
import { project, undo, useMock } from "./testUtils";

describe("MockTransport groove", () => {
  const f = useMock();

  it("humanize is deterministic for a seed", async () => {
    const chords = Object.values(project(f).clips).find((c) => c.name === "Chords")!;
    const humanize = cmd("Groove", { type: "Humanize", clip: chords.id, notes: null, timing: 0.05, velocity: 0.1, seed: 99 });
    const notesOf = () =>
      Object.values(project(f).notes)
        .filter((n) => n.clip === chords.id)
        .map((n) => [n.id, n.start, n.velocity]);
    const before = notesOf();
    await f.mock.send(humanize);
    const first = notesOf();
    expect(first).not.toEqual(before);
    await undo(f);
    expect(notesOf()).toEqual(before);
    await f.mock.send(humanize);
    expect(notesOf()).toEqual(first);
    await f.mock.send(cmd("Groove", { type: "SetSwing", amount: 2, grid: 0.5 }));
    expect(project(f).settings).toMatchObject({ swing: 1, swing_grid: 0.5 });
  });

  it("rejects non-finite amounts like the controller; NaN swing = straight", async () => {
    const chords = Object.values(project(f).clips).find((c) => c.name === "Chords")!;
    const before = project(f).notes;
    await expect(
      f.mock.send(cmd("Groove", { type: "Humanize", clip: chords.id, notes: null, timing: NaN, velocity: 0.1, seed: 1 })),
    ).rejects.toMatchObject({ code: "InvalidArgument" });
    await expect(f.mock.send(cmd("Groove", { type: "SetSwing", amount: NaN, grid: 0.5 }))).rejects.toMatchObject({
      code: "InvalidArgument",
    });
    expect(project(f).notes).toEqual(before);
    expect(swingOffset(0.5, 0.5, NaN)).toBe(0);
    expect(swingOffset(0.5, 0.5, 1)).toBeCloseTo(0.5 / 3, 12);
  });
});
