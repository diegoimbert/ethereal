/** MockTransport: `TimeEdit` (v0.2, time-edits), same behaviour as the controller tests. */
import { describe, expect, it } from "vitest";
import type { TrackId } from "@/generated";
import { cmd, type DomainCommand } from "../../cmd";
import { newId } from "../../ids";
import { project, undo, useMock } from "./testUtils";

describe("MockTransport TimeEdit (time-edits)", () => {
  const f = useMock();
  const seed = () => newId();

  async function midiTrack(clips: Array<[number, number]>): Promise<TrackId> {
    const id = newId();
    await f.mock.send(
      cmd("Track", {
        type: "Create",
        id,
        kind: "Midi",
        name: null,
        color: null,
        parent: null,
        before: null,
      }),
    );
    for (const [start, length] of clips) {
      await f.mock.send(
        cmd("Clip", {
          type: "CreateMidi",
          id: newId(),
          track: id,
          start,
          length,
          name: null,
        }),
      );
    }
    return id;
  }

  const spans = (track: TrackId) =>
    Object.values(project(f).clips)
      .filter((c) => c.track === track)
      .map((c) => [c.start, c.length, c.offset])
      .sort((a, b) => a[0]! - b[0]!);

  const timeEdit = (c: DomainCommand<"TimeEdit">) => f.mock.send(cmd("TimeEdit", c));

  it("splits across tracks as one undo step", async () => {
    const a = await midiTrack([[0, 8]]);
    const b = await midiTrack([[2, 4]]);
    await timeEdit({ type: "Split", tracks: [a, b], at: 4, seed: seed() });
    expect(spans(a)).toEqual([
      [0, 4, 0],
      [4, 4, 4],
    ]);
    expect(spans(b)).toEqual([
      [2, 2, 0],
      [4, 2, 2],
    ]);
    await undo(f);
    expect(spans(a)).toEqual([[0, 8, 0]]);
    expect(spans(b)).toEqual([[2, 4, 0]]);
  });

  it("deletes and inserts time, shifting later material", async () => {
    const t = await midiTrack([
      [0, 6],
      [8, 4],
    ]);
    await timeEdit({
      type: "DeleteTime",
      selection: { start: 4, end: 9, tracks: [t], global: false },
      seed: seed(),
    });
    expect(spans(t)).toEqual([
      [0, 4, 0],
      [4, 3, 1],
    ]);
    await timeEdit({
      type: "InsertSilence",
      tracks: [t],
      at: 2,
      length: 2,
      global: false,
      seed: seed(),
    });
    expect(spans(t)).toEqual([
      [0, 2, 0],
      [4, 2, 2],
      [6, 3, 1],
    ]);
  });

  it("global edits move markers; partial selections cannot be global", async () => {
    const t = await midiTrack([]);
    const marker = newId();
    await f.mock.send(
      cmd("Marker", {
        type: "Add",
        id: marker,
        position: 10,
        name: null,
        color: null,
      }),
    );
    await expect(
      timeEdit({
        type: "DeleteTime",
        selection: { start: 0, end: 2, tracks: [t], global: true },
        seed: seed(),
      }),
    ).rejects.toMatchObject({
      code: "InvalidArgument",
    });
    await timeEdit({
      type: "DeleteTime",
      selection: { start: 0, end: 2, tracks: [], global: true },
      seed: seed(),
    });
    expect(project(f).markers[marker]!.position).toBe(8);
  });

  it("copy, paste (overwrite and insert), cut and duplicate", async () => {
    const a = await midiTrack([[0, 4]]);
    const b = await midiTrack([[0, 16]]);
    await expect(
      timeEdit({
        type: "Paste",
        at: 0,
        tracks: [],
        insert: false,
        seed: seed(),
      }),
    ).rejects.toMatchObject({ code: "InvalidState" });
    await timeEdit({
      type: "Copy",
      selection: { start: 1, end: 3, tracks: [a], global: false },
    });
    await timeEdit({
      type: "Paste",
      at: 8,
      tracks: [b],
      insert: false,
      seed: seed(),
    });
    expect(spans(b)).toEqual([
      [0, 8, 0],
      [8, 2, 1],
      [10, 6, 10],
    ]);
    await timeEdit({
      type: "Paste",
      at: 0,
      tracks: [],
      insert: true,
      seed: seed(),
    });
    expect(spans(a)).toEqual([
      [0, 2, 1],
      [2, 4, 0],
    ]);
    await undo(f);
    await timeEdit({
      type: "Cut",
      selection: { start: 0, end: 2, tracks: [a], global: false },
      seed: seed(),
    });
    expect(spans(a)).toEqual([[0, 2, 2]]);
    await timeEdit({
      type: "DuplicateTime",
      selection: { start: 0, end: 2, tracks: [a], global: false },
      seed: seed(),
    });
    expect(spans(a)).toEqual([
      [0, 2, 2],
      [2, 2, 2],
    ]);
  });

  it("track automation keeps its values across a delete", async () => {
    const t = await midiTrack([]);
    const lane = newId();
    await f.mock.send(
      cmd("Automation", {
        type: "CreateLane",
        id: lane,
        owner: { type: "Track", track: t },
        target: { type: "TrackVolume", track: t },
      }),
    );
    await f.mock.send(
      cmd("Automation", {
        type: "AddPoints",
        lane,
        points: [
          { id: newId(), time: 0, value: 0, curve: { type: "Linear" } },
          { id: newId(), time: 12, value: 1, curve: { type: "Linear" } },
        ],
      }),
    );
    await timeEdit({
      type: "DeleteTime",
      selection: { start: 4, end: 9, tracks: [t], global: false },
      seed: seed(),
    });
    const pts = Object.values(project(f).automation_points)
      .filter((p) => p.lane === lane)
      .map((p) => [p.time, +p.value.toFixed(4)]);
    expect(pts).toEqual([
      [0, 0],
      [4, 0.3333],
      [4, 0.75],
      [7, 1],
    ]);
  });
});
