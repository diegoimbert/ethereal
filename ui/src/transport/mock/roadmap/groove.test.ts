/** MockTransport: groove (groove). */
import { describe, expect, it } from "vitest";
import { cmd } from "../../cmd";
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
});
