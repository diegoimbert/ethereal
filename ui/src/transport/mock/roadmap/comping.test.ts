/** MockTransport: `Take` (v0.2, comping), mirroring `crates/ether-controller/tests/comping.rs`. */
import { describe, expect, it } from "vitest";
import type { Event, RecordingEvent, TrackId } from "@/generated";
import { compOf, compPieces, deriveId, lanesOf } from "@/features/comping/model";
import { cmd } from "../../cmd";
import { MockTransport } from "../MockTransport";
import { project, trackNamed, undo, useMock, type MockFixture } from "./testUtils";

let n = 0;
const id = () => `01J9${String(++n).padStart(22, "0")}`;

async function lane(f: MockFixture, track: TrackId, name: string | null = null): Promise<string> {
  const l = id();
  await f.mock.send(cmd("Take", { type: "CreateLane", id: l, track, name, before: null }));
  return l;
}

async function swipe(f: MockFixture, track: TrackId, l: string, start: number, end: number) {
  const r = { id: id(), split_id: id() };
  await f.mock.send(cmd("Take", { type: "SetComp", ...r, track, lane: l, start, end }));
  return r;
}

async function takeClip(f: MockFixture, track: TrackId, l: string, start: number, length: number): Promise<string> {
  const c = id();
  await f.mock.send(cmd("Clip", { type: "CreateMidi", id: c, track, start, length, name: null }));
  await f.mock.send(cmd("Take", { type: "MoveToLane", clips: [c], lane: l }));
  return c;
}

const regions = (f: MockFixture, track: TrackId) => compOf(project(f), track).map((r) => [r.lane, r.start, r.end]);

describe("MockTransport Take (comping)", () => {
  const f = useMock();

  it("creates, renames, orders and removes lanes (one undo step each)", async () => {
    const bass = trackNamed(f, "Bass").id;
    const a = await lane(f, bass);
    const b = await lane(f, bass);
    const c = await lane(f, bass, "Best");
    const names = () => lanesOf(project(f), bass).map((l) => l.name);
    expect(names()).toEqual(["Take 1", "Take 2", "Best"]);
    await f.mock.send(cmd("Take", { type: "MoveLane", id: c, before: a }));
    expect(names()).toEqual(["Best", "Take 1", "Take 2"]);
    await f.mock.send(cmd("Take", { type: "RenameLane", id: b, name: "Keeper" }));
    expect(names()).toEqual(["Best", "Take 1", "Keeper"]);
    await undo(f);
    await undo(f);
    expect(names()).toEqual(["Take 1", "Take 2", "Best"]);
    await expect(f.mock.send(cmd("Take", { type: "RenameLane", id: b, name: " " }))).rejects.toMatchObject({ code: "InvalidArgument" });
    const clip = await takeClip(f, bass, a, 0, 4);
    await swipe(f, bass, a, 0, 4);
    await f.mock.send(cmd("Take", { type: "RemoveLane", id: a }));
    expect(project(f).clips[clip]).toBeUndefined();
    expect(regions(f, bass)).toEqual([]);
    await undo(f);
    expect(project(f).clips[clip]?.lane).toBe(a);
  });

  it("swipes trim, split and replace regions like the controller", async () => {
    const bass = trackNamed(f, "Bass").id;
    const a = await lane(f, bass);
    const b = await lane(f, bass);
    await swipe(f, bass, a, 0, 8);
    const { id: inner, split_id } = await swipe(f, bass, b, 2, 4);
    expect(regions(f, bass)).toEqual([
      [a, 0, 2],
      [b, 2, 4],
      [a, 4, 8],
    ]);
    expect(project(f).comp_regions[inner]).toBeDefined();
    expect(project(f).comp_regions[split_id]).toBeDefined();
    await undo(f);
    expect(regions(f, bass)).toEqual([[a, 0, 8]]);
    await f.mock.send(cmd("Take", { type: "ClearComp", track: bass, start: 1, end: 3, split_id: id() }));
    expect(regions(f, bass)).toEqual([
      [a, 0, 1],
      [a, 3, 8],
    ]);
    const other = trackNamed(f, "Keys").id;
    await expect(swipe(f, other, a, 0, 1)).rejects.toMatchObject({ code: "InvalidArgument" });
    await expect(swipe(f, bass, a, 3, 3)).rejects.toMatchObject({ code: "InvalidArgument" });
    const r = compOf(project(f), bass)[0]!;
    await expect(f.mock.send(cmd("Take", { type: "SetCrossfade", region: r.id, crossfade: 0.6 }))).rejects.toMatchObject({ code: "InvalidArgument" });
    await f.mock.send(cmd("Take", { type: "SetCrossfade", region: r.id, crossfade: 0.1 }));
    expect(project(f).comp_regions[r.id]!.crossfade).toBe(0.1);
  });

  it("flattens the comp into main-lane clips with derived ids", async () => {
    const bass = trackNamed(f, "Bass").id;
    const a = await lane(f, bass);
    const b = await lane(f, bass);
    const ca = await takeClip(f, bass, a, 0, 4);
    const cb = await takeClip(f, bass, b, 0, 4);
    for (const [clip, pitch] of [
      [ca, 60],
      [cb, 72],
    ] as const) {
      const notes = [0, 1, 2, 3].map((s) => ({ id: id(), pitch, velocity: 0.8, start: s, duration: 0.5 }));
      await f.mock.send(cmd("Note", { type: "Add", clip, notes }));
    }
    await swipe(f, bass, a, 0, 2);
    await swipe(f, bass, b, 2, 4);
    const pieces = compPieces(project(f), bass);
    expect(pieces.map((p) => [p.clip, p.start, p.end, p.offset])).toEqual([
      [ca, 0, 2, 0],
      [cb, 2, 4, 2],
    ]);
    const seed = "01J8Z3Q4R5S6T7V8W9XAYBZC0D";
    const seedNotes = id();
    const mainBefore = Object.values(project(f).clips).filter((c) => c.track === bass && c.lane == null).length;
    await f.mock.send(cmd("Take", { type: "Flatten", track: bass, seed, seed_notes: seedNotes, keep_lanes: false }));
    const p = project(f);
    const c0 = p.clips[deriveId(seed, 0)]!;
    const c1 = p.clips[deriveId(seed, 1)]!;
    expect([c0.start, c0.length, c0.offset, c0.lane]).toEqual([0, 2, 0, undefined]);
    expect([c1.start, c1.length, c1.offset]).toEqual([2, 2, 2]);
    expect(Object.values(p.notes).filter((x) => x.clip === c0.id).map((x) => [x.pitch, x.start])).toEqual([
      [60, 0],
      [60, 1],
    ]);
    expect(p.notes[deriveId(seedNotes, 0)]?.clip).toBe(c0.id);
    expect(lanesOf(p, bass)).toEqual([]);
    expect(p.clips[ca]).toBeUndefined();
    expect(Object.values(p.clips).filter((c) => c.track === bass && c.lane == null).length).toBe(mainBefore + 2);
    await undo(f);
    expect(compOf(project(f), bass)).toHaveLength(2);
  });

  it("audition is runtime: no undo step; deleting the track removes its takes", async () => {
    const bass = trackNamed(f, "Bass").id;
    const a = await lane(f, bass);
    await swipe(f, bass, a, 0, 4);
    const patches = f.events.filter((e) => e.type === "Patch").length;
    await f.mock.send(cmd("Take", { type: "Audition", track: bass, lane: a }));
    expect(f.events.filter((e) => e.type === "Patch").length).toBe(patches);
    await undo(f);
    expect(regions(f, bass)).toEqual([]);
    await f.mock.send(cmd("Track", { type: "Delete", id: bass }));
    expect(Object.keys(project(f).take_lanes)).toEqual([]);
  });

  it("derive_id matches the controller's", () => {
    // Pinned in crates/ether-controller/tests/comping.rs.
    const seed = "01J8Z3Q4R5S6T7V8W9XAYBZC0D";
    expect([0, 1, 2].map((i) => deriveId(seed, i))).toEqual(["01J8Z3Q4R5S6T1EFD3CS0GPJYM", "01J8Z3Q4R5S6T4KSN14Z5KR71Z", "01J8Z3Q4R5S6TEW19PDZ8M53YS"]);
  });
});

describe("mock loop recording makes takes", () => {
  it("each loop pass becomes a take lane; the newest plays", async () => {
    const t = new MockTransport({ timers: "manual", seed: 9 });
    const events: RecordingEvent[] = [];
    t.onEvent((e: Event) => {
      if (e.type === "Recording") events.push(e.event);
    });
    const p0 = await t.connect();
    const bass = Object.values(p0.tracks).find((x) => x.name === "Bass")!;
    const before = Object.values(p0.clips).filter((c) => c.track === bass.id).length;
    await t.send(cmd("Transport", { type: "SetLoopRegion", region: { start: 32, end: 34 } }));
    await t.send(cmd("Transport", { type: "SetLoopEnabled", enabled: true }));
    await t.send(cmd("Transport", { type: "Locate", position: 32 }));
    await t.send(cmd("Recording", { type: "Arm", track: bass.id, armed: true, exclusive: true }));
    await t.send(cmd("Recording", { type: "SetRecording", enabled: true }));
    t.tick(16 * 190); // ~3 s at 120 bpm: 6 beats = three passes over [32, 34)
    await t.send(cmd("Recording", { type: "SetRecording", enabled: false }));
    const stopped = events.at(-1);
    if (stopped?.type !== "Stopped") throw new Error("no Stopped");
    const reply = await t.send(cmd("Project", { type: "Get" }));
    if (reply.type !== "Project") throw new Error("no project");
    const p = reply.project;
    const lanes = lanesOf(p, bass.id);
    expect(lanes.length).toBe(stopped.clips.length);
    expect(lanes.length).toBeGreaterThanOrEqual(2);
    for (const c of stopped.clips) expect(p.clips[c]!.lane).toBeDefined();
    const comp = compOf(p, bass.id);
    // The newest pass plays from the loop start (a partial last pass leaves the rest to
    // the previous one).
    expect(comp[0]!.lane).toBe(lanes.at(-1)!.id);
    expect(comp[0]!.start).toBe(32);
    // Main-lane clips outside the range are untouched.
    expect(Object.values(p.clips).filter((c) => c.track === bass.id && c.lane == null).length).toBe(before);
    // One undo step.
    await t.send(cmd("Edit", { type: "Undo" }));
    const after = await t.send(cmd("Project", { type: "Get" }));
    if (after.type !== "Project") throw new Error("no project");
    expect(Object.keys(after.project.take_lanes)).toEqual([]);
    t.dispose();
  });
});
