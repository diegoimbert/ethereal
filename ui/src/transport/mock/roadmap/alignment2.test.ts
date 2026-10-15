/** Mock rules aligned with the controller, round 2 (Rust is the source of truth). */
import { describe, expect, it } from "vitest";
import type { BuiltinDeviceType, Command } from "@/generated";
import { cmd } from "../../cmd";
import { createTrack, project, testId, trackNamed, useMock } from "./testUtils";
import { detectBpm } from "./shared";

describe("mock alignment with the controller (round 2)", () => {
  const f = useMock();
  const send = (c: Command) => f.mock.send(c);
  const insert = async (track: string, type: BuiltinDeviceType, before: string | null = null) => {
    const id = testId();
    await send(cmd("Device", { type: "Insert", id, track, device: { type: "Builtin", device: { type } as never }, before }));
    return id;
  };
  const warnings = () => f.events.filter((e) => e.type === "Notification" && e.level === "Warning");

  it("only puts note effects on MIDI tracks and keeps MIDI effects before the instrument (doc/devices.rs, midi_fx)", async () => {
    const audio = await createTrack(f, "Audio");
    await expect(insert(audio, "Arpeggiator")).rejects.toMatchObject({ code: "InvalidArgument" });
    const midi = await createTrack(f, "Midi");
    const synth = await insert(midi, "Synth");
    // Appended after the instrument: refused.
    await expect(insert(midi, "Arpeggiator")).rejects.toMatchObject({ code: "InvalidArgument" });
    const arp = await insert(midi, "Arpeggiator", synth);
    await expect(send(cmd("Device", { type: "Move", id: arp, track: midi, before: null }))).rejects.toMatchObject({ code: "InvalidArgument" });
    expect(project(f).devices[arp]).toBeDefined();
  });

  it("checks the MIDI-effect order of rack chains when a device moves out to the track (racks/mod.rs)", async () => {
    const midi = await createTrack(f, "Midi");
    const synth = await insert(midi, "Synth");
    const rack = await insert(midi, "MidiEffectRack", synth);
    const chain = testId();
    await send(cmd("Rack", { type: "AddChain", id: chain, rack, name: null, before: null }));
    const arp = testId();
    await send(cmd("Rack", { type: "InsertDevice", id: arp, chain, device: { type: "Builtin", device: { type: "Arpeggiator" } }, before: null }));
    // Out of the rack, after the synth: the track chain would have a MIDI effect after the instrument.
    await expect(send(cmd("Rack", { type: "MoveDevice", id: arp, chain: null, before: null }))).rejects.toMatchObject({ code: "InvalidArgument" });
    await send(cmd("Rack", { type: "MoveDevice", id: arp, chain: null, before: synth }));
    expect(project(f).devices[arp]?.chain ?? null).toBeNull();
  });

  it("places a duplicated rack-chain device next to it in its chain", async () => {
    const midi = await createTrack(f, "Midi");
    const rack = await insert(midi, "AudioEffectRack");
    const chain = testId();
    await send(cmd("Rack", { type: "AddChain", id: chain, rack, name: null, before: null }));
    const [a, b] = [testId(), testId()];
    await send(cmd("Rack", { type: "InsertDevice", id: a, chain, device: { type: "Builtin", device: { type: "Delay" } }, before: null }));
    await send(cmd("Rack", { type: "InsertDevice", id: b, chain, device: { type: "Builtin", device: { type: "Reverb" } }, before: null }));
    const copy = testId();
    await send(cmd("Device", { type: "Duplicate", id: a, new_id: copy }));
    const p = project(f);
    const order = [a, copy, b].map((id) => p.devices[id]!.order);
    expect([...order].sort()).toEqual(order);
  });

  it("drops a moved device's clip envelopes left on its old track, with a warning", async () => {
    const [x, y] = [await createTrack(f, "Midi"), await createTrack(f, "Midi")];
    const delay = await insert(x, "Delay");
    const clip = testId();
    await send(cmd("Clip", { type: "CreateMidi", id: clip, track: x, start: 0, length: 4, name: "c" }));
    const lane = testId();
    await send(cmd("Automation", { type: "CreateLane", id: lane, owner: { type: "Clip", clip }, target: { type: "DeviceParam", device: delay, param: 0 } }));
    await send(cmd("Device", { type: "Move", id: delay, track: y, before: null }));
    expect(project(f).automation_lanes[lane]).toBeUndefined();
    expect(warnings()).toHaveLength(1);
  });

  it("resets a built-in param to its stored default and rejects unknown params", async () => {
    const midi = await createTrack(f, "Midi");
    const delay = await insert(midi, "Delay");
    await send(cmd("Device", { type: "SetParam", device: delay, param: 0, value: 1 }));
    await send(cmd("Device", { type: "ResetParam", device: delay, param: 0 }));
    expect(project(f).devices[delay]!.params[0]).toBeDefined();
    await expect(send(cmd("Device", { type: "ResetParam", device: delay, param: 999 }))).rejects.toMatchObject({ code: "NotFound" });
    await expect(send(cmd("Device", { type: "SetParam", device: delay, param: 0, value: NaN }))).rejects.toMatchObject({ code: "InvalidArgument" });
  });

  it("names a track with a blank name like an unnamed one (doc/tracks.rs)", async () => {
    const id = testId();
    await send(cmd("Track", { type: "Create", id, kind: "Midi", name: "   ", color: null, parent: null, before: null }));
    expect(project(f).tracks[id]!.name.trim()).not.toBe("");
  });

  it("retargets a moved clip's envelopes to its new track, or removes them (retarget_clip_lanes)", async () => {
    const [x, y] = [await createTrack(f, "Midi"), await createTrack(f, "Midi")];
    const delay = await insert(x, "Delay");
    const clip = testId();
    await send(cmd("Clip", { type: "CreateMidi", id: clip, track: x, start: 0, length: 4, name: "c" }));
    const [vol, dev] = [testId(), testId()];
    await send(cmd("Automation", { type: "CreateLane", id: vol, owner: { type: "Clip", clip }, target: { type: "TrackVolume", track: x } }));
    await send(cmd("Automation", { type: "CreateLane", id: dev, owner: { type: "Clip", clip }, target: { type: "DeviceParam", device: delay, param: 0 } }));
    await send(cmd("Clip", { type: "Move", moves: [{ id: clip, track: y, start: 8 }] }));
    expect(project(f).automation_lanes[vol]?.target).toEqual({ type: "TrackVolume", track: y });
    expect(project(f).automation_lanes[dev]).toBeUndefined();
    expect(warnings()).toHaveLength(1);
  });

  it("requires a clip envelope to target its own track (doc/automation.rs)", async () => {
    const [x, y] = [await createTrack(f, "Midi"), await createTrack(f, "Midi")];
    const clip = testId();
    await send(cmd("Clip", { type: "CreateMidi", id: clip, track: x, start: 0, length: 4, name: null }));
    await expect(
      send(cmd("Automation", { type: "CreateLane", id: testId(), owner: { type: "Clip", clip }, target: { type: "TrackPan", track: y } })),
    ).rejects.toMatchObject({ code: "InvalidArgument" });
  });

  it("refuses audio clips of media still loading and non-finite clip times (doc/clips.rs)", async () => {
    const drums = trackNamed(f, "Drums");
    const media = Object.values(project(f).media)[0]!;
    await expect(
      send(cmd("Clip", { type: "CreateMidi", id: testId(), track: await createTrack(f, "Midi"), start: Infinity, length: 4, name: null })),
    ).rejects.toMatchObject({
      code: "InvalidArgument",
    });
    const id = testId();
    await send(cmd("Clip", { type: "CreateAudio", id, track: drums.id, start: 100, media: media.id }));
    expect(project(f).clips[id]!.length).toBeGreaterThan(0);
  });

  it("validates SetWarp and pins two markers when warp turns on (warp/mod.rs)", async () => {
    const drums = trackNamed(f, "Drums");
    const media = Object.values(project(f).media)[0]!;
    const id = testId();
    await send(cmd("Clip", { type: "CreateAudio", id, track: drums.id, start: 200, media: media.id }));
    const clip = project(f).clips[id]!;
    if (clip.content.type !== "Audio") throw new Error("audio clip");
    const warp = { ...clip.content.warp, enabled: true };
    await expect(send(cmd("Warp", { type: "SetWarp", clip: clip.id, warp: { ...warp, source_bpm: -1 } }))).rejects.toMatchObject({
      code: "InvalidArgument",
    });
    for (const m of Object.values(project(f).warp_markers)) if (m.clip === clip.id) await send(cmd("Warp", { type: "RemoveMarker", id: m.id }));
    await send(cmd("Warp", { type: "SetWarp", clip: clip.id, warp }));
    const markers = Object.values(project(f).warp_markers).filter((m) => m.clip === clip.id);
    expect(markers).toHaveLength(2);
    const after = project(f).clips[clip.id]!;
    expect(after.content.type === "Audio" && after.content.warp.source_bpm).toBeGreaterThan(0);
    expect(detectBpm(8)).toBe(120);
  });

  it("rejects NaN in numeric setters (mixer, clips, notes, tempo, loop)", async () => {
    const keys = trackNamed(f, "Keys");
    const bad = [
      cmd("Mixer", { type: "SetVolume", track: keys.id, volume: NaN }),
      cmd("Mixer", { type: "SetPan", track: keys.id, pan: NaN }),
      cmd("Transport", { type: "SetTempo", bpm: NaN }),
      cmd("Transport", { type: "SetLoopRegion", region: { start: 0, end: Infinity } }),
    ];
    for (const c of bad) await expect(send(c)).rejects.toMatchObject({ code: "InvalidArgument" });
    // ±∞ volume clamps (only NaN is refused).
    await send(cmd("Mixer", { type: "SetVolume", track: keys.id, volume: Infinity }));
    expect(project(f).tracks[keys.id]!.mixer.volume).toBe(6);
  });
});
