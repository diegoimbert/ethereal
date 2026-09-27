import { describe, expect, it } from "vitest";
import type { Event, LiveAudioChunk, LiveMidiNote, RecordingEvent } from "@/generated";
import { cmd } from "../../cmd";
import { MockTransport } from "../MockTransport";
import { synthesizePeaks } from "../peaks";
import { LIVE_FRAMES_PER_PEAK } from "./liveRecord";

describe("mock recording with live view", () => {
  it("emits throttled Progress while recording, then commits real clips", async () => {
    const t = new MockTransport({ timers: "manual", seed: 5 });
    const events: RecordingEvent[] = [];
    const order: string[] = [];
    t.onEvent((e: Event) => {
      if (e.type === "Recording") {
        events.push(e.event);
        order.push(e.event.type);
      } else if (e.type === "Patch") order.push("Patch");
      else if (e.type === "Media" && e.event.type === "PeaksReady") order.push("PeaksReady");
    });
    const project = await t.connect();
    const tracks = Object.values(project.tracks);
    const drums = tracks.find((x) => x.name === "Drums")!;
    const keys = tracks.find((x) => x.name === "Keys")!;
    await t.send(cmd("Transport", { type: "Locate", position: 8 }));
    await t.send(cmd("Recording", { type: "Arm", track: drums.id, armed: true, exclusive: true }));
    await t.send(cmd("Recording", { type: "Arm", track: keys.id, armed: true, exclusive: false }));
    await t.send(cmd("Recording", { type: "SetRecording", enabled: true }));
    expect(events.at(-1)).toMatchObject({ type: "Started" });
    t.tick(16 * 3 * 22); // ~1.06 s at 120 bpm, just over 2 beats

    const progress = events.filter((e): e is Extract<RecordingEvent, { type: "Progress" }> => e.type === "Progress");
    expect(progress.length).toBeGreaterThanOrEqual(15);
    expect(progress.length).toBeLessThanOrEqual(22);
    const chunks: LiveAudioChunk[] = progress.flatMap((p) => p.audio);
    const notes: LiveMidiNote[] = progress.flatMap((p) => p.midi);
    // Contiguous peaks from the take start.
    let next = 0;
    for (const c of chunks) {
      expect(c.track).toBe(drums.id);
      expect(c.start).toBe(8);
      expect(c.first_peak).toBe(next);
      next += c.min.length;
    }
    expect(next * LIVE_FRAMES_PER_PEAK).toBeGreaterThan(40_000);
    // A note on beat 9 and 10 (start, then end half a beat later).
    expect(notes.filter((n) => n.track === keys.id).map((n) => [n.start, n.length])).toEqual([
      [9, null],
      [9, 0.5],
      [10, null],
    ]);

    await t.send(cmd("Recording", { type: "SetRecording", enabled: false }));
    const stopped = events.at(-1);
    if (stopped?.type !== "Stopped") throw new Error("no Stopped");
    expect(stopped.clips).toHaveLength(2);
    expect(order.slice(-3)).toEqual(["Patch", "PeaksReady", "Stopped"]);
    const reply = await t.send(cmd("Project", { type: "Get" }));
    if (reply.type !== "Project") throw new Error("no project");
    const audio = reply.project.clips[stopped.clips[0]!]!;
    expect(audio.track).toBe(drums.id);
    expect(audio.start).toBe(8);
    if (audio.content.type !== "Audio") throw new Error("not audio");
    // The committed media's peaks are the live ones.
    const media = reply.project.media[audio.content.media]!;
    const peaks = synthesizePeaks(media, { media: media.id, samples_per_peak: LIVE_FRAMES_PER_PEAK, start_frame: 0, frame_count: next * LIVE_FRAMES_PER_PEAK });
    expect(peaks.max[0]).toEqual(chunks.flatMap((c) => c.max));
    const midi = reply.project.clips[stopped.clips[1]!]!;
    expect(midi.track).toBe(keys.id);
    // Nothing after Stopped.
    const n = events.length;
    t.tick(200);
    expect(events.length).toBe(n);
    t.dispose();
  });

  it("finishes the recording when the transport stops", async () => {
    const t = new MockTransport({ timers: "manual", seed: 6 });
    const kinds: string[] = [];
    t.onEvent((e) => {
      if (e.type === "Recording") kinds.push(e.event.type);
    });
    const project = await t.connect();
    const drums = Object.values(project.tracks).find((x) => x.name === "Drums")!;
    await t.send(cmd("Recording", { type: "Arm", track: drums.id, armed: true, exclusive: true }));
    await t.send(cmd("Recording", { type: "SetRecording", enabled: true }));
    t.tick(300);
    await t.send(cmd("Transport", { type: "Stop" }));
    expect(kinds.at(-1)).toBe("Stopped");
    t.dispose();
  });
});
