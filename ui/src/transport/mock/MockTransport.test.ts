import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { ClipStateChange, Event, Patch, PlayheadFrame, Project, Track } from "@/generated";
import { EMPTY_HISTORY, useProjectStore } from "@/state/projectStore";
import { clipsOfTrack, devicesOfTrack, notesOfClip, scenesOrdered, tracksOrdered } from "@/state/selectors";
import { cmd } from "../cmd";
import { CommandFailedError } from "../EngineTransport";
import { newId, nextGestureId } from "../ids";
import { MockTransport } from "./MockTransport";

/** A connected mock with manual timers, mirroring events into the real project store. */
async function setup(opts: { latencyMs?: number } = {}) {
  const mock = new MockTransport({ timers: "manual", seed: 42, ...opts });
  const events: Event[] = [];
  const store = useProjectStore.getState;
  store().reset();
  mock.onEvent((e) => {
    events.push(e);
    if (e.type === "Patch") store().applyPatch(e.patch);
    if (e.type === "ProjectLoaded") store().loadProject(e.project);
    if (e.type === "Transport") store().setTransport(e.state);
    if (e.type === "Session") store().applySessionChanges(e.changes);
  });
  const project = await mock.connect();
  store().loadProject(project);
  const patches = () => events.filter((e): e is Extract<Event, { type: "Patch" }> => e.type === "Patch").map((e) => e.patch);
  const p = (): Project => store().project!;
  const trackNamed = (name: string): Track => Object.values(p().tracks).find((t) => t.name === name)!;
  return { mock, events, patches, p, trackNamed, store };
}

let mock: MockTransport | undefined;
afterEach(() => mock?.dispose());

describe("MockTransport documents", () => {
  let env: Awaited<ReturnType<typeof setup>>;
  beforeEach(async () => {
    env = await setup();
    mock = env.mock;
  });

  it("connect returns the demo project and emits the transport state", () => {
    expect(env.p().settings.name).toBe("Demo");
    expect(env.events.some((e) => e.type === "Transport")).toBe(true);
    expect(env.store().transport?.bpm).toBe(120);
  });

  it("create track → Patch upsert delivered before the reply", async () => {
    const id = newId();
    const order: string[] = [];
    env.mock.onEvent((e) => order.push(e.type));
    const reply = await env.mock
      .send(cmd("Track", { type: "Create", id, kind: "Midi", name: null, color: null, parent: null, before: null }))
      .then((v) => {
        order.push("reply");
        return v;
      });
    expect(reply).toEqual({ type: "Unit" });
    expect(order).toEqual(["Patch", "reply"]);
    const last = env.patches().at(-1)!;
    expect(last.changes).toEqual([{ type: "Upsert", entity: { type: "Track", value: expect.objectContaining({ id, kind: "Midi" }) } }]);
    expect(last.history).toMatchObject({ can_undo: true, undo_label: "Create" });
    // Mirror updated; the new track sits before the return track (Ableton layout).
    expect(tracksOrdered(env.p()).map((t) => t.name)).toEqual(["Keys", "Bass", "Drums", "3 MIDI", "A Delay", "Master"]);
    expect(env.p().tracks[id]).toEqual(env.mock.snapshot().tracks[id]);
  });

  it("revisions increase by one per patch", async () => {
    const keys = env.trackNamed("Keys");
    await env.mock.send(cmd("Track", { type: "Rename", id: keys.id, name: "A" }));
    await env.mock.send(cmd("Track", { type: "Rename", id: keys.id, name: "B" }));
    const revs = env.patches().map((pt) => pt.revision);
    expect(revs).toEqual([1, 2]);
    expect(env.store().revision).toBe(2);
  });

  it("set volume with a gesture then undo reverts in one step", async () => {
    const keys = env.trackNamed("Keys");
    const original = keys.mixer.volume;
    const g = nextGestureId();
    for (const v of [-5, -8, -12]) {
      await env.mock.send(cmd("Mixer", { type: "SetVolume", track: keys.id, volume: v }), { gesture: g });
    }
    await env.mock.send(cmd("Edit", { type: "EndGesture", gesture: g }));
    expect(env.p().tracks[keys.id]!.mixer.volume).toBe(-12);
    expect(env.store().history).toMatchObject({ can_undo: true, undo_label: "Set Volume" });

    await env.mock.send(cmd("Edit", { type: "Undo" }));
    expect(env.p().tracks[keys.id]!.mixer.volume).toBe(original);
    expect(env.store().history).toEqual({ ...EMPTY_HISTORY, can_redo: true, redo_label: "Set Volume" });

    await env.mock.send(cmd("Edit", { type: "Redo" }));
    expect(env.p().tracks[keys.id]!.mixer.volume).toBe(-12);
  });

  it("a new gesture after EndGesture is a separate undo step", async () => {
    const keys = env.trackNamed("Keys");
    const g = nextGestureId();
    await env.mock.send(cmd("Mixer", { type: "SetVolume", track: keys.id, volume: -5 }), { gesture: g });
    await env.mock.send(cmd("Edit", { type: "EndGesture", gesture: g }));
    await env.mock.send(cmd("Mixer", { type: "SetVolume", track: keys.id, volume: -9 }), { gesture: g });
    await env.mock.send(cmd("Edit", { type: "Undo" }));
    expect(env.p().tracks[keys.id]!.mixer.volume).toBe(-5);
  });

  it("delete track cascades clips, notes, devices, sends and lanes via the patch", async () => {
    const keys = env.trackNamed("Keys");
    const before = env.p();
    const clipIds = Object.values(before.clips).filter((c) => c.track === keys.id).map((c) => c.id);
    const noteIds = Object.values(before.notes).filter((n) => clipIds.includes(n.clip)).map((n) => n.id);
    const deviceIds = devicesOfTrack(before, keys.id).map((d) => d.id);
    expect(clipIds.length).toBe(2);
    expect(noteIds.length).toBe(24);
    expect(deviceIds.length).toBe(2);

    await env.mock.send(cmd("Track", { type: "Delete", id: keys.id }));
    const removed = env.patches().at(-1)!.changes.filter((c) => c.type === "Remove").map((c) => (c.type === "Remove" ? c.key : null));
    const removedIds = new Set(removed.map((k) => k!.id));
    for (const id of [keys.id, ...clipIds, ...noteIds, ...deviceIds]) expect(removedIds.has(id)).toBe(true);

    const p = env.p();
    expect(p.tracks[keys.id]).toBeUndefined();
    expect(Object.values(p.clips).some((c) => c.track === keys.id)).toBe(false);
    expect(Object.values(p.notes).some((n) => clipIds.includes(n.clip))).toBe(false);
    expect(Object.values(p.sends)).toHaveLength(0);
    expect(Object.values(p.automation_lanes)).toHaveLength(0);
    expect(Object.values(p.automation_points)).toHaveLength(0);

    // And undo brings everything back.
    await env.mock.send(cmd("Edit", { type: "Undo" }));
    expect(env.p()).toEqual(before);
  });

  it("refuses to delete the master track and leaves the document unchanged", async () => {
    const master = env.trackNamed("Master");
    const err = await env.mock.send(cmd("Track", { type: "Delete", id: master.id })).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(CommandFailedError);
    expect((err as CommandFailedError).code).toBe("InvalidArgument");
    expect(env.patches()).toHaveLength(0);
  });

  it("adds and edits notes", async () => {
    const keys = env.trackNamed("Keys");
    const clip = clipsOfTrack(env.p(), keys.id)[0]!;
    const id = newId();
    await env.mock.send(cmd("Note", { type: "Add", clip: clip.id, notes: [{ id, pitch: 72, velocity: 0.5, start: 1, duration: 0.5 }] }));
    expect(env.p().notes[id]).toMatchObject({ clip: clip.id, pitch: 72, start: 1, duration: 0.5, muted: false });
    expect(notesOfClip(env.p(), clip.id)).toHaveLength(17);

    await env.mock.send(
      cmd("Note", { type: "Edit", edits: [{ id, pitch: 74, velocity: null, start: 2, duration: null, muted: null }] }),
    );
    expect(env.p().notes[id]).toMatchObject({ pitch: 74, start: 2, duration: 0.5, velocity: 0.5 });
    const last = env.patches().at(-1)!;
    expect(last.changes).toHaveLength(1);

    const err = await env.mock.send(cmd("Note", { type: "Add", clip: clip.id, notes: [{ id: newId(), pitch: 200, velocity: 1, start: 0, duration: 1 }] })).catch((e: unknown) => e);
    expect((err as CommandFailedError).code).toBe("InvalidArgument");
  });

  it("batch = one atomic undo step", async () => {
    const trackId = newId();
    const deviceId = newId();
    const clipId = newId();
    await env.mock.send(
      cmd("Edit", {
        type: "Batch",
        label: "Add Instrument Track",
        commands: [
          cmd("Track", { type: "Create", id: trackId, kind: "Midi", name: "Lead", color: null, parent: null, before: null }),
          cmd("Device", { type: "Insert", id: deviceId, track: trackId, device: { type: "Builtin", device: { type: "Synth" } }, before: null }),
          cmd("Clip", { type: "CreateMidi", id: clipId, track: trackId, location: { type: "Arrangement", start: 0 }, length: 4, name: null }),
        ],
      }),
    );
    expect(env.patches()).toHaveLength(1);
    expect(env.store().history.undo_label).toBe("Add Instrument Track");
    expect(env.p().devices[deviceId]?.track).toBe(trackId);

    await env.mock.send(cmd("Edit", { type: "Undo" }));
    expect(env.p().tracks[trackId]).toBeUndefined();
    expect(env.p().devices[deviceId]).toBeUndefined();
    expect(env.p().clips[clipId]).toBeUndefined();
    expect(env.store().history.can_undo).toBe(false);

    // A failing command inside a batch rolls the whole batch back.
    const before = env.mock.snapshot();
    const err = await env.mock
      .send(
        cmd("Edit", {
          type: "Batch",
          label: "Broken",
          commands: [
            cmd("Track", { type: "Create", id: newId(), kind: "Audio", name: null, color: null, parent: null, before: null }),
            cmd("Track", { type: "Rename", id: newId(), name: "missing" }),
          ],
        }),
      )
      .catch((e: unknown) => e);
    expect((err as CommandFailedError).code).toBe("NotFound");
    expect(env.mock.snapshot()).toEqual(before);
  });

  it("device params are clamped and reset to default", async () => {
    const keys = env.trackNamed("Keys");
    const synth = devicesOfTrack(env.p(), keys.id)[0]!;
    await env.mock.send(cmd("Device", { type: "SetParam", device: synth.id, param: 2, value: 99999 }));
    expect(env.p().devices[synth.id]!.params[2]).toBe(20000);
    await env.mock.send(cmd("Device", { type: "ResetParam", device: synth.id, param: 2 }));
    expect(env.p().devices[synth.id]!.params[2]).toBeUndefined();
    const desc = await env.mock.send(cmd("Device", { type: "GetDescriptor", device: synth.id }));
    expect(desc.type === "Descriptor" && desc.descriptor.name).toBe("Synth");
  });

  it("clips moved over others trim them Ableton-style", async () => {
    const drums = env.trackNamed("Drums");
    const clip = clipsOfTrack(env.p(), drums.id)[0]!; // [0, 16)
    const media = Object.values(env.p().media)[0]!;
    const id = newId();
    await env.mock.send(cmd("Clip", { type: "CreateAudio", id, track: drums.id, location: { type: "Arrangement", start: 12 }, media: media.id }));
    expect(env.p().clips[clip.id]!.length).toBe(12);
    expect(env.p().clips[id]!.length).toBe(16); // 8 s at 120 BPM
  });

  it("save Json → open Json round-trips the project", async () => {
    const keys = env.trackNamed("Keys");
    await env.mock.send(cmd("Project", { type: "SetName", name: "Round Trip" }));
    await env.mock.send(cmd("Mixer", { type: "SetPan", track: keys.id, pan: 0.25 }));
    const saved = await env.mock.send(cmd("Project", { type: "Save", target: { type: "Json" } }));
    expect(saved.type).toBe("Saved");
    const json = saved.type === "Saved" ? saved.json! : "";
    expect(JSON.parse(json)).toMatchObject({ format: "ethereal-project", version: 1 });
    const snapshot = env.mock.snapshot();

    await env.mock.send(cmd("Project", { type: "New" }));
    expect(env.p().settings.name).toBe("Untitled");
    expect(Object.values(env.p().tracks).map((t) => t.kind)).toEqual(["Master"]);

    const opened = await env.mock.send(cmd("Project", { type: "Open", source: { type: "Json", json } }));
    expect(opened.type === "Project" && opened.project).toEqual(snapshot);
    expect(env.p()).toEqual(snapshot);
    expect(env.events.filter((e) => e.type === "ProjectLoaded")).toHaveLength(2);
    expect(env.store().history.can_undo).toBe(false);

    const bad = await env.mock.send(cmd("Project", { type: "Open", source: { type: "Json", json: "{}" } })).catch((e: unknown) => e);
    expect((bad as CommandFailedError).code).toBe("Decode");
  });

  it("unsupported commands reply Err Unsupported", async () => {
    const err = await env.mock.send(cmd("Recording", { type: "SetRecording", enabled: true })).catch((e: unknown) => e);
    expect((err as CommandFailedError).code).toBe("Unsupported");
    expect(await env.mock.send(cmd("Plugin", { type: "List" }))).toEqual({ type: "Plugins", plugins: [] });
  });

  it("synthesizes deterministic peaks", async () => {
    const media = Object.values(env.p().media)[0]!;
    const request = { media: media.id, samples_per_peak: 512, start_frame: 0, frame_count: 44100 };
    const a = await env.mock.send(cmd("Media", { type: "GetPeaks", request }));
    const b = await env.mock.send(cmd("Media", { type: "GetPeaks", request }));
    expect(a).toEqual(b);
    if (a.type !== "Peaks") throw new Error("expected peaks");
    expect(a.peaks.max).toHaveLength(2);
    expect(a.peaks.max[0]).toHaveLength(Math.ceil(44100 / 512));
    expect(a.peaks.max[0]!.every((v) => v >= 0 && v <= 1)).toBe(true);
  });
});

describe("MockTransport playback", () => {
  it("playhead advances with the injected clock and loops", async () => {
    const env = await setup();
    mock = env.mock;
    const frames: PlayheadFrame[] = [];
    env.mock.subscribePlayhead((f) => frames.push(f));

    await env.mock.send(cmd("Transport", { type: "Play" }));
    expect(env.store().transport?.playing).toBe(true);
    env.mock.tick(1000); // 1 s at 120 BPM = 2 beats
    expect(frames.at(-1)!.transport.position).toBeCloseTo(2, 6);
    expect(frames.at(-1)!.transport.seconds).toBeCloseTo(1, 6);
    expect(frames.length).toBeGreaterThan(50);

    await env.mock.send(cmd("Transport", { type: "SetLoopRegion", region: { start: 0, end: 4 } }));
    await env.mock.send(cmd("Transport", { type: "SetLoopEnabled", enabled: true }));
    env.mock.tick(1500); // → beat 5, wrapped to 1
    expect(env.mock.playheadPosition).toBeCloseTo(1, 6);

    await env.mock.send(cmd("Transport", { type: "Stop" }));
    const at = env.mock.playheadPosition;
    env.mock.tick(500);
    expect(env.mock.playheadPosition).toBe(at);
    expect(env.store().transport?.playing).toBe(false);
  });

  it("launching a session clip goes Queued then Playing at the next bar", async () => {
    const env = await setup();
    mock = env.mock;
    const changes: ClipStateChange[] = [];
    env.mock.onEvent((e) => e.type === "Session" && changes.push(...e.changes));
    const keys = env.trackNamed("Keys");
    const arp = Object.values(env.p().clips).find((c) => c.track === keys.id && c.location.type === "Session")!;

    await env.mock.send(cmd("Transport", { type: "Play" }));
    env.mock.tick(500); // beat 1
    await env.mock.send(cmd("Session", { type: "LaunchClip", clip: arp.id }));
    expect(changes).toEqual([{ track: keys.id, clip: arp.id, state: "Queued" }]);
    expect(env.store().sessionStates[arp.id]).toBe("Queued");

    env.mock.tick(1400); // beat 3.8: still queued
    expect(changes).toHaveLength(1);
    env.mock.tick(200); // beat 4.2: bar line crossed
    expect(changes.at(-1)).toEqual({ track: keys.id, clip: arp.id, state: "Playing" });
    expect(env.store().sessionStates[arp.id]).toBe("Playing");

    await env.mock.send(cmd("Session", { type: "StopAll" }));
    expect(changes.at(-1)!.state).toBe("Stopping");
    env.mock.tick(2000);
    expect(changes.at(-1)!.state).toBe("Stopped");
    expect(env.store().sessionStates[arp.id]).toBeUndefined();

    // Scenes: create, reorder, delete.
    const scenes = scenesOrdered(env.p());
    const sceneId = newId();
    await env.mock.send(cmd("Session", { type: "CreateScene", id: sceneId, name: null, before: scenes[0]!.id }));
    expect(scenesOrdered(env.p())[0]!.id).toBe(sceneId);
    await env.mock.send(cmd("Session", { type: "DeleteScene", id: scenes[0]!.id }));
    expect(env.p().clips[arp.id]).toBeUndefined();
  });

  it("emits meters that rise while playing", async () => {
    const env = await setup();
    mock = env.mock;
    let peak = 0;
    env.mock.subscribeMeters((f) => {
      const master = f.tracks.find((t) => t.track === env.trackNamed("Master").id);
      if (master) peak = Math.max(peak, master.peak[0]);
    });
    await env.mock.send(cmd("Transport", { type: "Play" }));
    env.mock.tick(500);
    expect(peak).toBeGreaterThan(0.05);
  });

  it("simulates latency but keeps patch-before-reply ordering", async () => {
    const env = await setup({ latencyMs: 5 });
    mock = env.mock;
    const keys = env.trackNamed("Keys");
    let patched: Patch | null = null;
    env.mock.onEvent((e) => {
      if (e.type === "Patch") patched = e.patch;
    });
    const pending = env.mock.send(cmd("Track", { type: "Rename", id: keys.id, name: "Later" }));
    expect(patched).toBeNull();
    await pending;
    expect(patched).not.toBeNull();
    expect(env.p().tracks[keys.id]!.name).toBe("Later");
  });
});
