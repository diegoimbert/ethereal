/** MockTransport: tempo map CRUD and metronome settings (tempo-metronome). */
import { describe, expect, it } from "vitest";
import { cmd } from "../../cmd";
import { CommandFailedError } from "../../EngineTransport";
import { newId } from "../../ids";
import { project, undo, useMock } from "./testUtils";

describe("MockTransport tempo", () => {
  const f = useMock();

  it("tempo map CRUD is undoable and protects beat 0", async () => {
    const id = newId();
    await f.mock.send(cmd("Tempo", { type: "AddTempoPoint", id, time: 16, bpm: 90, curve: "Step" }));
    expect(project(f).tempo_points[id]?.bpm).toBe(90);
    await f.mock.send(cmd("Tempo", { type: "EditTempoPoint", id, time: null, bpm: 2000, curve: "Linear" }));
    expect(project(f).tempo_points[id]).toMatchObject({ bpm: 999, curve: "Linear" });
    const zero = Object.values(project(f).tempo_points).find((t) => t.time === 0)!;
    await expect(f.mock.send(cmd("Tempo", { type: "RemoveTempoPoints", ids: [zero.id] }))).rejects.toBeInstanceOf(CommandFailedError);
    await undo(f);
    await undo(f);
    expect(project(f).tempo_points[id]).toBeUndefined();
    await f.mock.send(cmd("Tempo", { type: "SetMetronomeSettings", volume: 12, accent: false, sound: "Wood" }));
    expect(project(f).settings).toMatchObject({ metronome_volume: 6, metronome_accent: false, metronome_sound: "Wood" });
  });
});
