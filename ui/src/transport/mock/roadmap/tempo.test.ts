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

  it("rejects shared positions; signature changes may sit mid-bar (like the controller)", async () => {
    const a = newId();
    await f.mock.send(cmd("Tempo", { type: "AddTempoPoint", id: a, time: 8, bpm: 90, curve: "Step" }));
    const dup = cmd("Tempo", { type: "AddTempoPoint", id: newId(), time: 8, bpm: 60, curve: "Step" });
    await expect(f.mock.send(dup)).rejects.toBeInstanceOf(CommandFailedError);
    const zero = Object.values(project(f).tempo_points).find((t) => t.time === 0)!;
    const move0 = cmd("Tempo", { type: "EditTempoPoint", id: zero.id, time: 4, bpm: null, curve: null });
    await expect(f.mock.send(move0)).rejects.toBeInstanceOf(CommandFailedError);
    // A 7/8 change at beat 2.5, mid bar 1 of 4/4 (CONTRACTS.md §11.3).
    const ts = newId();
    await f.mock.send(cmd("Tempo", { type: "AddTimeSignature", id: ts, time: 2.5, signature: { numerator: 7, denominator: 8 } }));
    const three = newId();
    await f.mock.send(cmd("Tempo", { type: "AddTimeSignature", id: three, time: 9.25, signature: { numerator: 3, denominator: 4 } }));
    expect(Object.keys(project(f).time_signatures)).toHaveLength(3);
    const zeroSig = Object.values(project(f).time_signatures).find((s) => s.time === 0)!;
    const rejected = [
      cmd("Tempo", { type: "AddTimeSignature", id: newId(), time: 2.5, signature: { numerator: 5, denominator: 4 } }),
      cmd("Tempo", { type: "AddTimeSignature", id: newId(), time: -1, signature: { numerator: 5, denominator: 4 } }),
      cmd("Tempo", { type: "AddTimeSignature", id: newId(), time: 3, signature: { numerator: 5, denominator: 3 } }),
      cmd("Tempo", { type: "EditTimeSignature", id: ts, time: 9.25, signature: null }),
      cmd("Tempo", { type: "EditTimeSignature", id: zeroSig.id, time: 1, signature: null }),
      cmd("Tempo", { type: "RemoveTimeSignatures", ids: [zeroSig.id] }),
    ];
    const before = project(f).time_signatures;
    for (const c of rejected) await expect(f.mock.send(c)).rejects.toBeInstanceOf(CommandFailedError);
    expect(project(f).time_signatures).toEqual(before);
    // Edits that used to push a later change off its bar line are ordinary edits now.
    await f.mock.send(cmd("Tempo", { type: "EditTimeSignature", id: zeroSig.id, time: null, signature: { numerator: 3, denominator: 4 } }));
    await f.mock.send(cmd("Tempo", { type: "EditTimeSignature", id: ts, time: 1.75, signature: null }));
    expect(project(f).time_signatures[ts]).toMatchObject({ time: 1.75 });
    await undo(f);
    expect(project(f).time_signatures[ts]).toMatchObject({ time: 2.5 });
    await f.mock.send(cmd("Tempo", { type: "RemoveTimeSignatures", ids: [ts] }));
    expect(project(f).time_signatures[ts]).toBeUndefined();
  });
});
