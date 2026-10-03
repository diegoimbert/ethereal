/** MockTransport: `Capture::*` (v0.3, capture-midi): the always-on MIDI capture buffer. */
import { describe, expect, it } from "vitest";
import type { CaptureResult, ReplyValue } from "@/generated";
import { deriveId } from "@/features/comping/model";
import { cmd } from "../../cmd";
import { inferBpm } from "./capture";
import { createTrack, project, testId, undo, useMock, type MockFixture } from "./testUtils";

/** Play `(startMs, durationMs, key)` notes from now on, advancing the mock clock. */
function play(f: MockFixture, notes: Array<[number, number, number]>): void {
  const evs: Array<[number, [number, number, number]]> = [];
  for (const [s, d, k] of notes) {
    evs.push([s, [0x90, k, 100]]);
    evs.push([s + d, [0x80, k, 0]]);
  }
  evs.sort((a, b) => a[0] - b[0]);
  let now = 0;
  for (const [t, data] of evs) {
    if (t > now) f.mock.tick(t - now);
    now = Math.max(now, t);
    f.mock.simulateMidiInput("keys", data);
  }
}

async function capture(f: MockFixture, track: string, adopt_tempo: boolean): Promise<{ r: CaptureResult; clip: string; seed: string }> {
  const clip = testId();
  const seed = testId();
  const reply = (await f.mock.send(cmd("Capture", { type: "Capture", track, clip, seed_notes: seed, adopt_tempo }))) as ReplyValue;
  if (reply.type !== "Captured") throw new Error(`unexpected ${reply.type}`);
  return { r: reply.capture, clip, seed };
}

const changed = (f: MockFixture) => f.events.filter((e) => e.type === "Capture");

describe("MockTransport MIDI capture (capture-midi)", () => {
  const f = useMock();

  it("captures a stopped phrase at the playhead, adopting tempo and loop in one undo step", async () => {
    const track = await createTrack(f, "Midi");
    const tempoBefore = Object.values(project(f).tempo_points)[0]!.bpm;
    // 100 bpm quarter notes.
    play(f, [0, 1, 2, 3, 4, 5, 6, 7].map((i) => [i * 600, 300, 60 + i] as [number, number, number]));
    const { r, clip, seed } = await capture(f, track, true);
    expect(r.bpm).toBeCloseTo(100, 1);
    expect(r).toMatchObject({ clip, start: 0, length: 8, notes: 8 });
    const p = project(f);
    expect(p.clips[clip]).toMatchObject({ track, start: 0, length: 8 });
    expect(p.settings.loop_enabled).toBe(true);
    expect(p.settings.loop_region).toEqual({ start: 0, end: 8 });
    const notes = Object.values(p.notes)
      .filter((n) => n.clip === clip)
      .sort((a, b) => a.start - b.start);
    expect(notes.map((n) => n.id)).toEqual(notes.map((_, i) => deriveId(seed, i)));
    notes.forEach((n, i) => {
      expect(n.start).toBeCloseTo(i, 2);
      expect(n.duration).toBeCloseTo(0.5, 2);
    });
    await undo(f);
    expect(project(f).clips[clip]).toBeUndefined();
    expect(project(f).settings.loop_enabled).toBe(false);
    expect(Object.values(project(f).tempo_points)[0]!.bpm).toBe(tempoBefore);
  });

  it("keeps song positions while playing", async () => {
    const track = await createTrack(f, "Midi");
    await f.mock.send(cmd("Transport", { type: "Locate", position: 8 }));
    await f.mock.send(cmd("Transport", { type: "Play" }));
    // 120 bpm: +0.5 s = beat 9.
    play(f, [
      [500, 250, 60],
      [1000, 250, 62],
    ]);
    const { r, clip } = await capture(f, track, true);
    expect(r).toMatchObject({ start: 8, length: 4, notes: 2, bpm: null });
    const starts = Object.values(project(f).notes)
      .filter((n) => n.clip === clip)
      .map((n) => n.start)
      .sort((a, b) => a - b);
    expect(starts[0]).toBeCloseTo(1, 1);
    expect(starts[1]).toBeCloseTo(2, 1);
  });

  it("filters by the track input and records CC / bend lanes", async () => {
    const track = await createTrack(f, "Midi");
    await f.mock.send(cmd("Recording", { type: "SetInput", track, input: { type: "Midi", port: "keys", channel: 1 } }));
    f.mock.simulateMidiInput("keys", [0x90, 50, 100]); // channel 0: not this track
    await expect(f.mock.send(cmd("Capture", { type: "Capture", track, clip: testId(), seed_notes: testId(), adopt_tempo: false }))).rejects.toMatchObject({
      code: "InvalidState",
    });
    f.mock.simulateMidiInput("keys", [0xb1, 1, 127]);
    f.mock.simulateMidiInput("keys", [0x91, 60, 100]);
    f.mock.tick(500);
    f.mock.simulateMidiInput("keys", [0xe1, 0x7f, 0x7f]);
    f.mock.simulateMidiInput("keys", [0x81, 60, 0]);
    const { r, clip } = await capture(f, track, false);
    expect(r.notes).toBe(1);
    const lanes = Object.values(project(f).expression_lanes).filter((l) => l.clip === clip);
    expect(lanes.map((l) => l.kind)).toEqual([{ type: "Cc", controller: 1 }, { type: "PitchBend" }]);
    expect(lanes[1]!.points).toEqual([{ time: 1, value: 1, curve: { type: "Step" } }]);
  });

  it("Status, Clear and availability events", async () => {
    const track = await createTrack(f, "Midi");
    expect(await f.mock.send(cmd("Capture", { type: "Status" }))).toEqual({ type: "CaptureStatus", status: { available: false, notes: 0, seconds: 0 } });
    f.mock.simulateMidiInput("keys", [0x90, 60, 100]);
    f.mock.tick(1000);
    f.mock.simulateMidiInput("keys", [0x80, 60, 0]);
    f.mock.simulateMidiInput("keys", [0x90, 62, 100]);
    expect(changed(f)).toEqual([{ type: "Capture", event: { type: "Changed", status: { available: true, notes: 1, seconds: 0 } } }]);
    expect(await f.mock.send(cmd("Capture", { type: "Status" }))).toEqual({ type: "CaptureStatus", status: { available: true, notes: 2, seconds: 1 } });
    await f.mock.send(cmd("Capture", { type: "Clear" }));
    expect(changed(f)).toHaveLength(2);
    await expect(f.mock.send(cmd("Capture", { type: "Capture", track, clip: testId(), seed_notes: testId(), adopt_tempo: true }))).rejects.toMatchObject({
      code: "InvalidState",
    });
    // Capturing empties the buffer.
    f.mock.simulateMidiInput("keys", [0x90, 60, 100]);
    await capture(f, track, false);
    expect(changed(f)).toHaveLength(4);
    expect(((await f.mock.send(cmd("Capture", { type: "Status" }))) as { status: { available: boolean } }).status.available).toBe(false);
  });

  it("rejects non-MIDI tracks", async () => {
    const audio = await createTrack(f, "Audio");
    f.mock.simulateMidiInput("keys", [0x90, 60, 100]);
    await expect(f.mock.send(cmd("Capture", { type: "Capture", track: audio, clip: testId(), seed_notes: testId(), adopt_tempo: false }))).rejects.toMatchObject({
      code: "InvalidArgument",
    });
  });

  it("infers tempo like the controller", () => {
    const quarters = (bpm: number, n: number) => Array.from({ length: n }, (_, i) => (i * 60) / bpm);
    expect(inferBpm(quarters(100, 16))).toBeCloseTo(100, 1);
    expect(inferBpm(quarters(60, 8))).toBe(120);
    expect(inferBpm([1])).toBeNull();
  });
});
