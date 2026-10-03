import { describe, expect, it } from "vitest";
import type { Clip, Command, Track } from "@/generated";
import { dragPreview } from "./editMath";
import { laneItems } from "./laneItems";
import { layoutRows, rowsHeight, TRACK_HEIGHT } from "./layout";
import { inNewTrackZone, newTrackDropCommand, newTrackLanes, onNewLanes } from "./newTrackDrag";

function track(id: string, kind: Track["kind"], order: string, color = 0xff0000): Track {
  return {
    id,
    kind,
    name: id,
    color,
    order,
    parent: null,
    mixer: { volume: 0, pan: 0, mute: false, solo: false },
    input: { type: "None" },
    output: { type: "Default" },
    monitor: "Auto",
    scale: { type: "FollowProject" },
  };
}

function midi(id: string, trackId: string, start: number, length = 4): Clip {
  return {
    id,
    track: trackId,
    start,
    name: id,
    color: null,
    muted: false,
    length,
    offset: 0,
    looping: { enabled: false, start: 0, end: length },
    content: { type: "Midi" },
  };
}

const audio = (id: string, trackId: string, start: number): Clip => ({
  ...midi(id, trackId, start),
  content: { type: "Audio", media: "m1", gain: 0, transpose: 0, fade_in: 0, fade_out: 0 } as Clip["content"],
});

// Display order: m1 (MIDI), a1 (audio), m2 (MIDI); the master is pinned (not in `rows`).
const tracks = [track("m1", "Midi", "a", 0x111111), track("a1", "Audio", "b", 0x222222), track("m2", "Midi", "c", 0x333333)];
const rows = layoutRows(tracks, new Set());
const ids = () => {
  let n = 0;
  return () => `new${++n}`;
};
const snap = (b: number) => Math.round(b);

describe("inNewTrackZone", () => {
  it("is the empty canvas below the last track, above the pinned master", () => {
    const bottom = rowsHeight(rows);
    expect(bottom).toBe(3 * TRACK_HEIGHT);
    expect(inNewTrackZone(rows, bottom - 1)).toBe(false);
    expect(inNewTrackZone(rows, 0)).toBe(false);
    expect(inNewTrackZone(rows, bottom)).toBe(true);
    expect(inNewTrackZone(rows, bottom + 200, bottom + 300)).toBe(true);
    // At or past the bottom of the scrolling area: over the master, never a drop target.
    expect(inNewTrackZone(rows, bottom + 300, bottom + 300)).toBe(false);
    expect(inNewTrackZone([], 10)).toBe(false);
  });
});

describe("newTrackLanes", () => {
  it("makes one new track per source track, of its kind, in display order", () => {
    const clips = [midi("c3", "m2", 0), audio("c2", "a1", 0), midi("c1", "m1", 0), midi("c1b", "m1", 8)];
    const lanes = newTrackLanes(clips, rows, ids());
    expect(lanes.map((l) => [l.source, l.kind, l.color])).toEqual([
      ["m1", "Midi", 0x111111],
      ["a1", "Audio", 0x222222],
      ["m2", "Midi", 0x333333],
    ]);
    expect(new Set(lanes.map((l) => l.id)).size).toBe(3);
  });

  it("compacts gaps: tracks not involved don't get a lane", () => {
    const lanes = newTrackLanes([midi("c3", "m2", 0), midi("c1", "m1", 0)], rows, ids());
    expect(lanes.map((l) => l.source)).toEqual(["m1", "m2"]);
  });
});

describe("drop below the last track", () => {
  const clips = [midi("c1", "m1", 0), audio("c2", "a1", 2)];
  const lanes = newTrackLanes(clips, rows, ids());
  const preview = onNewLanes(dragPreview({ clips, anchor: "c1", rows, snap }, "move", 3.4), clips, lanes);

  it("previews each clip on its new lane at the snapped time (ghost lanes)", () => {
    expect(preview.get("c1")).toMatchObject({ track: lanes[0]!.id, start: 3 });
    expect(preview.get("c2")).toMatchObject({ track: lanes[1]!.id, start: 5 });
    const all = Object.fromEntries(clips.map((c) => [c.id, c]));
    const p = { bounds: preview, copy: false, newTracks: lanes };
    expect(laneItems(all, lanes[0]!.id, p).map((i) => i.clip.id)).toEqual(["c1"]);
    expect(laneItems(all, lanes[1]!.id, p).map((i) => i.clip.id)).toEqual(["c2"]);
    // A move takes the clips out of their own lanes.
    expect(laneItems(all, "m1", p)).toEqual([]);
  });

  it("move: creates the tracks (MIDI with the synth) then moves, as one batch", () => {
    const command = newTrackDropCommand(lanes, clips, preview, false, ids())!;
    expect(command.domain).toBe("Edit");
    const batch = (command as Extract<Command, { domain: "Edit" }>).command;
    if (batch.type !== "Batch") throw new Error("not a batch");
    expect(batch.label).toBe("Move Clips to New Tracks");
    const kinds = batch.commands.map((c) => `${c.domain}.${c.command.type}`);
    expect(kinds).toEqual(["Track.Create", "Device.Insert", "Track.Create", "Clip.Move"]);
    const created = batch.commands.filter((c) => c.domain === "Track").map((c) => c.command as { id: string; kind: string });
    expect(created.map((c) => [c.id, c.kind])).toEqual([
      [lanes[0]!.id, "Midi"],
      [lanes[1]!.id, "Audio"],
    ]);
    const move = batch.commands.at(-1)!.command as { moves: Array<{ id: string; track: string; start: number }> };
    expect(move.moves).toEqual([
      { id: "c1", track: lanes[0]!.id, start: 3 },
      { id: "c2", track: lanes[1]!.id, start: 5 },
    ]);
  });

  it("copy: duplicates parked on the source track, then moves the copies (still one batch)", () => {
    const command = newTrackDropCommand(lanes.slice(0, 1), clips.slice(0, 1), preview, true, ids())!;
    const batch = (command as Extract<Command, { domain: "Edit" }>).command;
    if (batch.type !== "Batch") throw new Error("not a batch");
    expect(batch.label).toBe("Copy Clip to New Track");
    expect(batch.commands.map((c) => `${c.domain}.${c.command.type}`)).toEqual([
      "Track.Create",
      "Device.Insert",
      "Clip.Duplicate",
      "Clip.Move",
    ]);
  });
});
