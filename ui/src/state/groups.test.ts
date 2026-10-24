import { beforeEach, describe, expect, it } from "vitest";
import type { Note, Patch } from "@/generated";
import { createDemoProject } from "@/transport/mock/demoProject";
import { EMPTY_HISTORY, useProjectStore } from "./projectStore";
import { clipsOfTrack, devicesOfTrack, notesOfClip, pointsOfLane } from "./selectors";
import { byOrder, byTime, clipOrder, noteOrder } from "./groups";

const store = () => useProjectStore.getState();
let rev = 0;
const patch = (changes: Patch["changes"]): Patch => ({ revision: ++rev, changes, history: EMPTY_HISTORY });

/** The groups a full regroup builds, for comparison. */
function fresh() {
  const p = store().project!;
  const notes = (clip: string) =>
    Object.values(p.notes)
      .filter((n) => n.clip === clip)
      .sort(noteOrder);
  const clips = (track: string) =>
    Object.values(p.clips)
      .filter((c) => c.track === track)
      .sort(clipOrder);
  const devices = (track: string) =>
    Object.values(p.devices)
      .filter((d) => d.pad === null && d.chain == null && d.track === track)
      .sort(byOrder);
  const points = (lane: string) =>
    Object.values(p.automation_points)
      .filter((pt) => pt.lane === lane)
      .sort(byTime);
  return { p, notes, clips, devices, points };
}

function expectCarriedEqualsFresh() {
  const f = fresh();
  for (const c of Object.keys(f.p.clips)) expect(notesOfClip(f.p, c)).toEqual(f.notes(c));
  for (const t of Object.keys(f.p.tracks)) {
    expect(clipsOfTrack(f.p, t)).toEqual(f.clips(t));
    expect(devicesOfTrack(f.p, t)).toEqual(f.devices(t));
  }
  for (const l of Object.keys(f.p.automation_lanes)) expect(pointsOfLane(f.p, l)).toEqual(f.points(l));
}

describe("carryGroups", () => {
  beforeEach(() => {
    store().reset();
    store().loadProject(createDemoProject());
    rev = 0;
  });

  it("matches a full regroup after edits, moves, removals and inserts, and keeps untouched groups", () => {
    let p = store().project!;
    // Build every group once (so there is something to carry).
    expectCarriedEqualsFresh();
    const midi = Object.values(p.clips).filter((c) => c.content.type === "Midi");
    const [a, b] = [midi[0]!, midi[1]!];
    const untouched = midi.slice(2).map((c) => notesOfClip(p, c.id));
    const notesA = notesOfClip(p, a.id);
    const n0 = notesA[0]!;
    const n1 = notesA[1]!;
    const added: Note = { ...n0, id: "01HZZZZZZZZZZZZZZZZZZZZZZZ", start: n0.start + 0.125 };
    const steps: Patch["changes"][] = [
      [{ type: "Upsert", entity: { type: "Note", value: { ...n0, start: n0.start + 3 } } }],
      [{ type: "Upsert", entity: { type: "Note", value: { ...n1, clip: b.id } } }],
      [{ type: "Remove", key: { type: "Note", id: n0.id } }],
      [{ type: "Upsert", entity: { type: "Note", value: added } }],
      [{ type: "Upsert", entity: { type: "Clip", value: { ...a, start: a.start + 8 } } }],
    ];
    const dev = Object.values(p.devices)[0];
    if (dev) steps.push([{ type: "DeviceParam", id: dev.id, param: 0, value: 0.25 }]);
    const pt = Object.values(p.automation_points)[0];
    if (pt) steps.push([{ type: "Upsert", entity: { type: "AutomationPoint", value: { ...pt, time: pt.time + 1 } } }]);
    for (const changes of steps) {
      expect(store().applyPatch(patch(changes))).toBe("applied");
      expectCarriedEqualsFresh();
    }
    p = store().project!;
    midi.slice(2).forEach((c, i) => expect(notesOfClip(p, c.id)).toBe(untouched[i]));
  });
});
