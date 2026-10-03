/** MockTransport: `History::*` (v0.3, undo-history; CONTRACTS.md §13.8). */
import { describe, expect, it } from "vitest";
import type { Event, HistoryList, ReplyValue } from "@/generated";
import { MockTransport } from "../MockTransport";
import { cmd } from "../../cmd";
import { createTrack, project, useMock } from "./testUtils";

const list = async (m: MockTransport): Promise<HistoryList> => {
  const r = (await m.send(cmd("History", { type: "List" }))) as Extract<ReplyValue, { type: "History" }>;
  return r.history;
};

const setVolume = (m: MockTransport, track: string, db: number) => m.send(cmd("Mixer", { type: "SetVolume", track, volume: db }));

const changed = (events: Event[]): HistoryList[] =>
  events.flatMap((e) => (e.type === "History" && e.event.type === "Changed" ? [e.event.history] : []));

describe("MockTransport undo history (undo-history)", () => {
  const f = useMock();

  it("lists the steps as a timeline", async () => {
    expect(await list(f.mock)).toEqual({ steps: [], current: null, truncated: false });
    const t = await createTrack(f, "Midi");
    f.mock.tick(1000);
    await setVolume(f.mock, t, -6);
    const l = await list(f.mock);
    expect(l.steps.map((s) => s.undone)).toEqual([false, false]);
    expect(l.steps[1]!.id).toBeGreaterThan(l.steps[0]!.id);
    expect(l.steps[1]!.time_ms - l.steps[0]!.time_ms).toBe(1000);
    expect(l.current).toBe(l.steps[1]!.id);
    await f.mock.send(cmd("Edit", { type: "Undo" }));
    const after = await list(f.mock);
    expect(after.steps.map((s) => s.undone)).toEqual([false, true]);
    expect(after.current).toBe(l.steps[0]!.id);
  });

  it("jumps like N undos/redos in one patch", async () => {
    const t = await createTrack(f, "Midi");
    for (const db of [-3, -6, -9]) await setVolume(f.mock, t, db);
    const ids = (await list(f.mock)).steps.map((s) => s.id);
    f.events.length = 0;
    await f.mock.send(cmd("History", { type: "JumpTo", step: ids[1]! }));
    expect(f.events.filter((e) => e.type === "Patch")).toHaveLength(1);
    expect(project(f).tracks[t]!.mixer.volume).toBe(-3);
    await f.mock.send(cmd("History", { type: "JumpTo", step: null }));
    expect(project(f).tracks[t]).toBeUndefined();
    await f.mock.send(cmd("History", { type: "JumpTo", step: ids[3]! }));
    expect(project(f).tracks[t]!.mixer.volume).toBe(-9);
    expect((await list(f.mock)).steps.map((s) => s.id)).toEqual(ids);
    await expect(f.mock.send(cmd("History", { type: "JumpTo", step: 999 }))).rejects.toMatchObject({ code: "NotFound" });
  });

  it("names checkpoints and pushes changes once listed", async () => {
    const t = await createTrack(f, "Midi");
    expect(changed(f.events)).toEqual([]);
    const [first] = (await list(f.mock)).steps;
    await f.mock.send(cmd("History", { type: "SetCheckpoint", step: first!.id, name: "  Start " }));
    expect(changed(f.events).at(-1)!.steps[0]!.checkpoint).toBe("Start");
    await setVolume(f.mock, t, -1);
    expect(changed(f.events).at(-1)!.steps).toHaveLength(2);
    await f.mock.send(cmd("History", { type: "SetCheckpoint", step: first!.id, name: null }));
    expect(changed(f.events).at(-1)!.steps[0]!.checkpoint).toBeNull();
    await expect(f.mock.send(cmd("History", { type: "SetCheckpoint", step: 999, name: "x" }))).rejects.toMatchObject({
      code: "NotFound",
    });
    await expect(
      f.mock.send(cmd("History", { type: "SetCheckpoint", step: first!.id, name: "a".repeat(500) })),
    ).rejects.toMatchObject({ code: "InvalidArgument" });
  });

  it("marks a truncated history", async () => {
    const m = new MockTransport({ timers: "manual", seed: 7, historyLimit: 2 });
    await m.connect();
    const t = "01K00000000000000000000099";
    await m.send(cmd("Track", { type: "Create", id: t, kind: "Midi", name: null, color: null, parent: null, before: null }));
    for (const db of [-1, -2]) await setVolume(m, t, db);
    const l = await list(m);
    expect(l.truncated).toBe(true);
    expect(l.steps).toHaveLength(2);
    m.dispose();
  });
});
