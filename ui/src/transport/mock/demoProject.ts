/**
 * Project factories for the MockTransport: an empty project (what `Project::New` creates)
 * and a small but complete demo project exercising every table, so UI nodes have real
 * data to render without the Rust engine.
 *
 * Ids are deterministic (seeded ULIDs), so tests and screenshots are stable.
 */

import type {
  AutomationLane,
  AutomationPoint,
  Clip,
  ClipContent,
  Color,
  Device,
  LaunchSettings,
  MediaRef,
  Note,
  Project,
  ProjectSettings,
  Scene,
  Track,
  TrackInput,
  TrackKind,
} from "@/generated";
import { keysBetween } from "@/state/orderKey";
import { builtinDescriptor } from "./builtinDevices";
import { seededIdFactory } from "./random";

/** Default track colors (0xRRGGBB), cycled for new tracks. */
export const MOCK_TRACK_COLORS: ReadonlyArray<Color> = [
  0xff764d, 0xffa53f, 0xf0d03f, 0x99d44a, 0x3fc98c, 0x3fc2d9, 0x5c9dff, 0x9b7bff, 0xe06adf, 0xff6b8b,
];

export const DEFAULT_LAUNCH: LaunchSettings = { mode: "Trigger", quantization: null, legato: false };

export function defaultSettings(name: string): ProjectSettings {
  return {
    name,
    loop_enabled: false,
    loop_region: { start: 0, end: 16 },
    metronome: false,
    launch_quantization: { type: "Bars", count: 1 },
    count_in_bars: 0,
  };
}

export function defaultTrackInput(kind: TrackKind): TrackInput {
  switch (kind) {
    case "Audio":
      return { type: "Audio", first: 0, count: 2 };
    case "Midi":
      return { type: "Midi", port: null, channel: null };
    default:
      return { type: "None" };
  }
}

export function defaultTrackName(kind: TrackKind, n: number): string {
  switch (kind) {
    case "Audio":
      return `${n} Audio`;
    case "Midi":
      return `${n} MIDI`;
    case "Group":
      return `${n} Group`;
    case "Return":
      return `${String.fromCharCode(64 + Math.max(1, n))} Return`;
    case "Master":
      return "Master";
  }
}

export function makeTrack(fields: Pick<Track, "id" | "kind" | "name" | "color" | "order"> & Partial<Track>): Track {
  return {
    parent: null,
    mixer: { volume: 0, pan: 0, mute: false, solo: false },
    input: defaultTrackInput(fields.kind),
    output: { type: "Master" },
    arm: false,
    monitor: "Auto",
    ...fields,
  };
}

export function makeClip(fields: Pick<Clip, "id" | "track" | "location" | "length"> & Partial<Clip>): Clip {
  const content: ClipContent = fields.content ?? { type: "Midi" };
  return {
    name: "",
    color: null,
    muted: false,
    offset: 0,
    looping: { enabled: fields.location.type === "Session", start: 0, end: fields.length },
    launch: DEFAULT_LAUNCH,
    content,
    ...fields,
  };
}

export function makeScene(id: string, name: string, order: string): Scene {
  return { id, name, color: null, order, tempo: null, time_signature: null };
}

/** Default param values of a built-in device (plain units). */
export function defaultParams(device: Parameters<typeof builtinDescriptor>[0]): Record<number, number> {
  const params: Record<number, number> = {};
  for (const p of builtinDescriptor(device).params) params[p.id] = p.default;
  return params;
}

/**
 * A new empty project: master track, tempo 120 and 4/4 at beat 0, 4 empty scenes, default
 * settings. `nextId` generates entity ids.
 */
export function createEmptyProject(nextId: () => string, name = "Untitled"): Project {
  const master = makeTrack({ id: nextId(), kind: "Master", name: "Master", color: 0x9a9a9a, order: "a0" });
  const tempoId = nextId();
  const sigId = nextId();
  const sceneKeys = keysBetween(null, null, 4);
  const scenes = sceneKeys.map((order, i) => makeScene(nextId(), `${i + 1}`, order));
  return {
    id: nextId(),
    settings: defaultSettings(name),
    tracks: { [master.id]: master },
    clips: {},
    notes: {},
    devices: {},
    sends: {},
    scenes: Object.fromEntries(scenes.map((s) => [s.id, s])),
    automation_lanes: {},
    automation_points: {},
    tempo_points: { [tempoId]: { id: tempoId, time: 0, bpm: 120, curve: "Step" } },
    time_signatures: { [sigId]: { id: sigId, time: 0, signature: { numerator: 4, denominator: 4 } } },
    warp_markers: {},
    media: {},
  };
}

/**
 * The demo project:
 * - tracks (in order): "Keys" (MIDI, Synth, 4-bar clip with 16 notes, a session clip in
 *   scene 1, a volume automation lane with 3 points, a send to the return), "Bass" (MIDI,
 *   4-bar clip), "Drums" (audio, 8 s stereo 44.1 kHz loop clip), "A Delay" (return, Delay
 *   device), "Master";
 * - 4 scenes, tempo 120 at 0, 4/4 at 0, settings name "Demo".
 */
export function createDemoProject(seed = 1): Project {
  const nextId = seededIdFactory(seed);
  const p = createEmptyProject(nextId, "Demo");
  const scenes = Object.values(p.scenes).sort((a, b) => (a.order < b.order ? -1 : 1));
  const master = Object.values(p.tracks)[0]!;

  // Track order: Keys, Bass, Drums, A Delay, Master (master last, as in Ableton).
  const [kKeys, kBass, kDrums, kReturn, kMaster] = keysBetween(null, null, 5) as [string, string, string, string, string];
  master.order = kMaster;

  const keys = makeTrack({ id: nextId(), kind: "Midi", name: "Keys", color: MOCK_TRACK_COLORS[6]!, order: kKeys });
  const bass = makeTrack({ id: nextId(), kind: "Midi", name: "Bass", color: MOCK_TRACK_COLORS[1]!, order: kBass });
  const drums = makeTrack({ id: nextId(), kind: "Audio", name: "Drums", color: MOCK_TRACK_COLORS[3]!, order: kDrums });
  const ret = makeTrack({ id: nextId(), kind: "Return", name: "A Delay", color: MOCK_TRACK_COLORS[8]!, order: kReturn });
  keys.mixer.volume = -3;
  bass.mixer.volume = -4.5;
  bass.mixer.pan = -0.1;
  drums.mixer.volume = -2;
  for (const t of [keys, bass, drums, ret]) p.tracks[t.id] = t;

  // Devices.
  const [d1, d2] = keysBetween(null, null, 2) as [string, string];
  const synth: Device = {
    id: nextId(),
    track: keys.id,
    order: d1,
    name: "Synth",
    enabled: true,
    kind: { type: "Builtin", device: { type: "Synth" } },
    params: { ...defaultParams("Synth"), 0: 1, 2: 2400, 3: 25, 7: 450 },
  };
  const keysComp: Device = {
    id: nextId(),
    track: keys.id,
    order: d2,
    name: "Compressor",
    enabled: true,
    kind: { type: "Builtin", device: { type: "Compressor" } },
    params: defaultParams("Compressor"),
  };
  const bassSynth: Device = {
    id: nextId(),
    track: bass.id,
    order: d1,
    name: "Synth",
    enabled: true,
    kind: { type: "Builtin", device: { type: "Synth" } },
    params: { ...defaultParams("Synth"), 0: 2, 2: 600, 8: -3 },
  };
  const delay: Device = {
    id: nextId(),
    track: ret.id,
    order: d1,
    name: "Delay",
    enabled: true,
    kind: { type: "Builtin", device: { type: "Delay" } },
    params: { ...defaultParams("Delay"), 4: 100 },
  };
  for (const d of [synth, keysComp, bassSynth, delay]) p.devices[d.id] = d;

  // Send Keys → return.
  const send = { id: nextId(), from: keys.id, to: ret.id, level: -12, pre_fader: false };
  p.sends[send.id] = send;

  // Keys: 4-bar chord clip (Am - F - C - G), 16 notes.
  const keysClip = makeClip({ id: nextId(), track: keys.id, location: { type: "Arrangement", start: 0 }, length: 16, name: "Chords" });
  p.clips[keysClip.id] = keysClip;
  const chords = [
    [57, 60, 64, 69],
    [53, 57, 60, 65],
    [55, 60, 64, 67],
    [55, 59, 62, 67],
  ];
  chords.forEach((chord, bar) => {
    chord.forEach((pitch, i) => {
      const n: Note = {
        id: nextId(),
        clip: keysClip.id,
        pitch,
        velocity: 0.7 + 0.05 * i,
        release_velocity: 0.5,
        start: bar * 4,
        duration: 3.5,
        muted: false,
      };
      p.notes[n.id] = n;
    });
  });

  // Keys: session clip in scene 1 (1 bar arpeggio, looping).
  const arp = makeClip({ id: nextId(), track: keys.id, location: { type: "Session", scene: scenes[0]!.id }, length: 4, name: "Arp" });
  p.clips[arp.id] = arp;
  [69, 72, 76, 72, 69, 72, 76, 79].forEach((pitch, i) => {
    const n: Note = { id: nextId(), clip: arp.id, pitch, velocity: 0.8, release_velocity: 0.5, start: i * 0.5, duration: 0.45, muted: false };
    p.notes[n.id] = n;
  });

  // Bass: 4-bar clip starting at bar 5.
  const bassClip = makeClip({ id: nextId(), track: bass.id, location: { type: "Arrangement", start: 16 }, length: 16, name: "Bassline" });
  p.clips[bassClip.id] = bassClip;
  [33, 33, 29, 29, 36, 36, 31, 31].forEach((pitch, i) => {
    const n: Note = { id: nextId(), clip: bassClip.id, pitch, velocity: 0.9, release_velocity: 0.5, start: i * 2, duration: 1.5, muted: false };
    p.notes[n.id] = n;
  });

  // Drums: audio clip over a fake 8 s stereo file (16 beats at 120 BPM).
  const media: MediaRef = {
    id: nextId(),
    name: "drum-loop-120.wav",
    location: { type: "ProjectRelative", path: "Samples/drum-loop-120.wav" },
    sample_rate: 44100,
    channels: 2,
    frames: 44100 * 8,
    hash: null,
  };
  p.media[media.id] = media;
  const drumClip = makeClip({
    id: nextId(),
    track: drums.id,
    location: { type: "Arrangement", start: 0 },
    length: 16,
    name: "drum-loop-120",
    content: {
      type: "Audio",
      media: media.id,
      gain: 0,
      transpose: 0,
      fade_in: 0,
      fade_out: 0,
      warp: { enabled: true, mode: "Repitch", source_bpm: 120 },
    },
  });
  p.clips[drumClip.id] = drumClip;

  // Keys volume automation: 3 points (normalized values).
  const lane: AutomationLane = {
    id: nextId(),
    owner: { type: "Track", track: keys.id },
    target: { type: "TrackVolume", track: keys.id },
    enabled: true,
  };
  p.automation_lanes[lane.id] = lane;
  const points: AutomationPoint[] = [
    { id: nextId(), lane: lane.id, time: 0, value: 0.7, curve: { type: "Linear" } },
    { id: nextId(), lane: lane.id, time: 8, value: 0.9, curve: { type: "Linear" } },
    { id: nextId(), lane: lane.id, time: 16, value: 0.6, curve: { type: "Linear" } },
  ];
  for (const pt of points) p.automation_points[pt.id] = pt;

  return p;
}
