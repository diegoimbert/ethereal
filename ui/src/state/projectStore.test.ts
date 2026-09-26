import { beforeEach, describe, expect, it } from "vitest";
import type { Patch, Track } from "@/generated";
import { createDemoProject } from "@/transport/mock/demoProject";
import { EMPTY_HISTORY, useProjectStore } from "./projectStore";
import { clipsOfTrack, devicesOfTrack, notesOfClip, pointsOfLane, tracksOrdered } from "./selectors";

const store = () => useProjectStore.getState();

function patch(revision: number, changes: Patch["changes"]): Patch {
  return { revision, changes, history: { ...EMPTY_HISTORY, can_undo: true, undo_label: "Test" } };
}

describe("projectStore.applyPatch", () => {
  beforeEach(() => {
    store().reset();
    store().loadProject(createDemoProject());
  });

  it("upserts and removes entities by type", () => {
    const keys = tracksOrdered(store().project!).find((t) => t.name === "Keys")!;
    const renamed: Track = { ...keys, name: "Piano" };
    expect(store().applyPatch(patch(1, [{ type: "Upsert", entity: { type: "Track", value: renamed } }]))).toBe("applied");
    expect(store().project!.tracks[keys.id]!.name).toBe("Piano");
    expect(store().revision).toBe(1);
    expect(store().history.undo_label).toBe("Test");

    const note = Object.values(store().project!.notes)[0]!;
    store().applyPatch(patch(2, [{ type: "Remove", key: { type: "Note", id: note.id } }]));
    expect(store().project!.notes[note.id]).toBeUndefined();

    store().applyPatch(patch(3, [{ type: "Settings", settings: { ...store().project!.settings, name: "Renamed" } }]));
    expect(store().project!.settings.name).toBe("Renamed");
  });

  it("keeps untouched entities referentially stable", () => {
    const before = store().project!;
    const [a, b] = Object.values(before.tracks) as [Track, Track];
    store().applyPatch(patch(1, [{ type: "Upsert", entity: { type: "Track", value: { ...a, name: "X" } } }]));
    const after = store().project!;
    expect(after).not.toBe(before);
    expect(after.tracks[b.id]).toBe(b);
    expect(after.clips).toBe(before.clips);
  });

  it("ignores stale revisions and reports gaps", () => {
    const t = Object.values(store().project!.tracks)[0]!;
    const up = (name: string) => [{ type: "Upsert" as const, entity: { type: "Track" as const, value: { ...t, name } } }];
    expect(store().applyPatch(patch(5, up("five")))).toBe("applied"); // first patch after load sets the revision
    expect(store().applyPatch(patch(5, up("dup")))).toBe("stale");
    expect(store().applyPatch(patch(4, up("old")))).toBe("stale");
    expect(store().project!.tracks[t.id]!.name).toBe("five");
    expect(store().applyPatch(patch(7, up("gap")))).toBe("gap");
    expect(store().project!.tracks[t.id]!.name).toBe("five");
    expect(store().applyPatch(patch(6, up("six")))).toBe("applied");
  });

  it("does nothing without a project", () => {
    store().reset();
    expect(store().applyPatch(patch(1, []))).toBe("no-project");
  });
});

describe("selectors on the demo project", () => {
  const p = createDemoProject();

  it("orders tracks, devices, clips, notes and points", () => {
    expect(tracksOrdered(p).map((t) => t.name)).toEqual(["Keys", "Bass", "Drums", "A Delay", "Master"]);
    const keys = tracksOrdered(p)[0]!;
    expect(devicesOfTrack(p, keys.id).map((d) => d.name)).toEqual(["Synth", "Compressor"]);
    expect(clipsOfTrack(p, keys.id)).toHaveLength(1);
    const clip = clipsOfTrack(p, keys.id)[0]!;
    const notes = notesOfClip(p, clip.id);
    expect(notes).toHaveLength(16);
    expect(notes.map((n) => n.start)).toEqual([...notes.map((n) => n.start)].sort((a, b) => a - b));
    const lane = Object.values(p.automation_lanes)[0]!;
    expect(pointsOfLane(p, lane.id).map((pt) => pt.time)).toEqual([0, 8, 16]);
  });
});
