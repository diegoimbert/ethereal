import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { Event, Patch, PlayheadFrame, Project, Track } from "@/generated";
import { EMPTY_HISTORY, useProjectStore } from "@/state/projectStore";
import { clipsOfTrack, devicesOfTrack, notesOfClip, tracksOrdered } from "@/state/selectors";
import { cmd } from "../cmd";
import { CommandFailedError } from "../EngineTransport";
import { newId, newProjectId, nextGestureId } from "../ids";
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
    if (e.type === "Recording" && e.event.type === "ArmChanged") store().setArmedTracks(e.event.armed);
    if (e.type === "Project") {
      if (e.event.type === "ListChanged") store().setProjects(e.event.projects);
      if (e.event.type === "Saved") store().upsertProjectSummary(e.event.project);
      if (e.event.type === "DirtyChanged") store().setDirty(e.event.dirty);
    }
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
    // Patch, then Project::DirtyChanged (first edit since load), then the reply.
    expect(order).toEqual(["Patch", "Project", "reply"]);
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
    expect(clipIds.length).toBe(1);
    expect(noteIds.length).toBe(16);
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
          cmd("Clip", { type: "CreateMidi", id: clipId, track: trackId, start: 0, length: 4, name: null }),
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
    await env.mock.send(cmd("Clip", { type: "CreateAudio", id, track: drums.id, start: 12, media: media.id }));
    expect(env.p().clips[clip.id]!.length).toBe(12);
    expect(env.p().clips[id]!.length).toBe(16); // 8 s at 120 BPM
  });

  it("connect reports the engine-side project list, dirty flag and armed tracks", () => {
    expect(env.store().projects.map((pr) => pr.name)).toEqual(["Demo", "Beat sketch", "Ambient idea"]);
    expect(env.store().projects[0]!.id).toBe(env.p().id);
    expect(env.p().id).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
    expect(env.store().dirty).toBe(false);
    expect(env.store().armedTracks).toEqual([]);
  });

  it("save / open / create / save-as / duplicate / rename / delete use the engine-side store", async () => {
    const demoId = env.p().id;
    const keys = env.trackNamed("Keys");
    await env.mock.send(cmd("Mixer", { type: "SetPan", track: keys.id, pan: 0.25 }));
    expect(env.store().dirty).toBe(true);

    // Rename of the current project: undoable document edit + list update.
    await env.mock.send(cmd("Project", { type: "Rename", id: demoId, name: "Round Trip" }));
    expect(env.p().settings.name).toBe("Round Trip");
    expect(env.store().projects.find((pr) => pr.id === demoId)?.name).toBe("Round Trip");
    expect(env.store().history.undo_label).toBe("Rename Project");

    const saved = await env.mock.send(cmd("Project", { type: "Save" }));
    expect(saved).toMatchObject({ type: "Saved", project: { id: demoId, name: "Round Trip" } });
    expect(env.store().dirty).toBe(false);
    const snapshot = env.mock.snapshot();

    // Create switches to a new empty project.
    const newIdP = newProjectId();
    const created = await env.mock.send(cmd("Project", { type: "Create", id: newIdP, name: "Fresh" }));
    expect(created.type === "Project" && created.project.id).toBe(newIdP);
    expect(env.p().settings.name).toBe("Fresh");
    expect(Object.values(env.p().tracks).map((t) => t.kind)).toEqual(["Master"]);
    expect(env.store().projects.map((pr) => pr.name)).toContain("Fresh");

    // Open brings back exactly what was saved.
    await env.mock.send(cmd("Project", { type: "Open", id: demoId }));
    expect(env.p()).toEqual(snapshot);
    expect(env.store().history.can_undo).toBe(false);

    // Unsaved changes are autosaved when opening another project.
    await env.mock.send(cmd("Mixer", { type: "SetMute", track: keys.id, mute: true }));
    await env.mock.send(cmd("Project", { type: "Open", id: newIdP }));
    await env.mock.send(cmd("Project", { type: "Open", id: demoId }));
    expect(env.p().tracks[keys.id]!.mixer.mute).toBe(true);

    // Duplicate a stored project without opening it; rename and delete stored projects.
    const beat = env.store().projects.find((pr) => pr.name === "Beat sketch")!;
    const dupId = newProjectId();
    const dup = await env.mock.send(cmd("Project", { type: "Duplicate", id: beat.id, new_id: dupId, name: "Beat copy" }));
    expect(dup).toMatchObject({ type: "Saved", project: { id: dupId, name: "Beat copy" } });
    expect(env.p().id).toBe(demoId);
    await env.mock.send(cmd("Project", { type: "Rename", id: dupId, name: "Beat v2" }));
    expect(env.store().projects.find((pr) => pr.id === dupId)?.name).toBe("Beat v2");
    await env.mock.send(cmd("Project", { type: "Delete", id: dupId }));
    expect(env.store().projects.some((pr) => pr.id === dupId)).toBe(false);

    const busy = await env.mock.send(cmd("Project", { type: "Delete", id: demoId })).catch((e: unknown) => e);
    expect((busy as CommandFailedError).code).toBe("InvalidState");
    const missing = await env.mock.send(cmd("Project", { type: "Open", id: newProjectId() })).catch((e: unknown) => e);
    expect((missing as CommandFailedError).code).toBe("NotFound");

    // Save As switches to the copy; the original stays in the store.
    const asId = newProjectId();
    await env.mock.send(cmd("Project", { type: "SaveAs", new_id: asId, name: "Variation" }));
    expect(env.p().id).toBe(asId);
    expect(env.p().settings.name).toBe("Variation");
    expect(env.mock.storedProjects().map((pr) => pr.id)).toEqual(expect.arrayContaining([demoId, asId]));
  });

  it("record-arm is runtime state reported via ArmChanged, not a patch", async () => {
    const keys = env.trackNamed("Keys");
    const bass = env.trackNamed("Bass");
    await env.mock.send(cmd("Recording", { type: "Arm", track: keys.id, armed: true, exclusive: false }));
    await env.mock.send(cmd("Recording", { type: "Arm", track: bass.id, armed: true, exclusive: false }));
    expect(env.store().armedTracks).toEqual([keys.id, bass.id]);
    await env.mock.send(cmd("Recording", { type: "Arm", track: bass.id, armed: true, exclusive: true }));
    expect(env.store().armedTracks).toEqual([bass.id]);
    expect(env.patches()).toHaveLength(0);
    expect(env.store().history.can_undo).toBe(false);
    // Deleting an armed track disarms it.
    await env.mock.send(cmd("Track", { type: "Delete", id: bass.id }));
    expect(env.store().armedTracks).toEqual([]);
  });

  it("group tracks nest via parent; Default output routes into the group; delete cascades", async () => {
    const group = newId();
    const child = newId();
    await env.mock.send(cmd("Track", { type: "Create", id: group, kind: "Group", name: "Bus", color: null, parent: null, before: null }));
    await env.mock.send(cmd("Track", { type: "Create", id: child, kind: "Audio", name: "Gtr", color: null, parent: group, before: null }));
    const keys = env.trackNamed("Keys");
    await env.mock.send(cmd("Track", { type: "Move", id: keys.id, parent: group, before: child }));
    expect(tracksOrdered(env.p()).map((t) => t.name)).toEqual(["Bass", "Drums", "Bus", "Keys", "Gtr", "A Delay", "Master"]);
    expect(env.p().tracks[child]!.output).toEqual({ type: "Default" });

    const cycle = await env.mock.send(cmd("Track", { type: "Move", id: group, parent: group, before: null })).catch((e: unknown) => e);
    expect((cycle as CommandFailedError).code).toBe("InvalidArgument");

    await env.mock.send(cmd("Track", { type: "Delete", id: group }));
    expect(env.p().tracks[child]).toBeUndefined();
    expect(env.p().tracks[keys.id]).toBeUndefined();
  });

  it("browses the library and imports media into the project", async () => {
    const locs = await env.mock.send(cmd("Media", { type: "ListLocations" }));
    expect(locs.type === "Locations" && locs.locations.map((l) => l.name)).toEqual(["Library", "Project media"]);
    const lib = locs.type === "Locations" ? locs.locations[0]!.location : null;
    const root = await env.mock.send(cmd("Media", { type: "ListDirectory", location: lib!, path: "" }));
    expect(root.type === "Directory" && root.listing.entries.filter((e) => e.kind === "Directory").map((e) => e.name)).toEqual([
      "Drums",
      "MIDI",
      "Synths",
      "Vocals",
    ]);
    const drums = await env.mock.send(cmd("Media", { type: "ListDirectory", location: lib!, path: "Drums" }));
    const kick = drums.type === "Directory" ? drums.listing.entries.find((e) => e.name === "Kick.wav")! : null;
    expect(kick).toMatchObject({ path: "Drums/Kick.wav", kind: "Audio" });
    expect(kick!.size).toBeGreaterThan(0);

    const id = newId();
    const reply = await env.mock.send(cmd("Media", { type: "Import", id, source: { type: "Location", location: lib!, path: kick!.path } }));
    expect(reply.type === "Media" && reply.media).toMatchObject({ id, name: "Kick.wav", channels: 1, sample_rate: 44100 });
    expect(env.p().media[id]?.file).toBe(`media/${id}-Kick.wav`);
    expect(env.patches().at(-1)!.changes).toEqual([{ type: "Upsert", entity: { type: "Media", value: env.p().media[id] } }]);

    const pm = await env.mock.send(cmd("Media", { type: "ListDirectory", location: { type: "ProjectMedia" }, path: "" }));
    expect(pm.type === "Directory" && pm.listing.entries.map((e) => e.name)).toContain(`${id}-Kick.wav`);

    // Uploads are staged since `file-import` (`roadmap/remote.test.ts`).
    const upload = await env.mock.send(cmd("Media", { type: "BeginUpload", upload: "u1", name: "x.wav", size: 10 }));
    expect(upload).toEqual({ type: "Unit" });
    const missing = await env.mock
      .send(cmd("Media", { type: "Import", id: newId(), source: { type: "Location", location: lib!, path: "Nope.wav" } }))
      .catch((e: unknown) => e);
    expect((missing as CommandFailedError).code).toBe("NotFound");
  });

  it("unsupported commands reply Err Unsupported", async () => {
    const err = await env.mock
      .send(
        cmd("Engine", {
          type: "SetAudioConfig",
          config: { backend: null, host: null, output_device: null, input_device: null, sample_rate: null, buffer_size: null },
        }),
      )
      .catch((e: unknown) => e);
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
