/**
 * Project factories for the MockTransport: an empty project (what `Project::Create`
 * makes), a small but complete demo project exercising every table, and the 3 projects
 * the mock's engine-side project store starts with ("Demo", "Beat sketch",
 * "Ambient idea"), so UI nodes have real data to render without the Rust engine.
 *
 * Ids are deterministic (seeded ULIDs / UUIDv7s), so tests and screenshots are stable.
 */

import type {
  AutomationLane,
  AutomationPoint,
  Clip,
  ClipContent,
  Color,
  Device,
  MediaRef,
  Note,
  Project,
  ProjectId,
  ProjectSettings,
  Track,
  TrackInput,
  TrackKind,
} from "@/generated";
import { keysBetween } from "@/state/orderKey";
import { builtinDescriptor } from "./builtinDevices";
import { seededIdFactory, seededProjectId } from "./random";

/** Default track colors (0xRRGGBB), cycled for new tracks. */
export const MOCK_TRACK_COLORS: ReadonlyArray<Color> = [
  0xe8919d, 0xe8a585, 0xe6c07e, 0xa9cf8b, 0x86cfa8, 0x7cc6c0, 0x86bfe0, 0x8fa8e6, 0xbd9ae3, 0xd99adf,
];

export function defaultSettings(name: string): ProjectSettings {
  return {
    name,
    loop_enabled: false,
    loop_region: { start: 0, end: 16 },
    metronome: false,
    count_in_bars: 0,
    metronome_volume: -6,
    metronome_accent: true,
    metronome_sound: "Classic",
    swing: 0,
    swing_grid: 0.25,
    scale: { root: 0, kind: "Chromatic" },
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
    case "Vca":
      return `${n} VCA`;
  }
}

export function makeTrack(fields: Pick<Track, "id" | "kind" | "name" | "color" | "order"> & Partial<Track>): Track {
  return {
    parent: null,
    mixer: { volume: 0, pan: 0, mute: false, solo: false },
    input: defaultTrackInput(fields.kind),
    output: { type: "Default" },
    monitor: "Auto",
    scale: { type: "FollowProject" },
    ...fields,
  };
}

export function makeClip(fields: Pick<Clip, "id" | "track" | "start" | "length"> & Partial<Clip>): Clip {
  const content: ClipContent = fields.content ?? { type: "Midi" };
  return {
    name: "",
    color: null,
    muted: false,
    offset: 0,
    looping: { enabled: false, start: 0, end: fields.length },
    content,
    ...fields,
  };
}

/** Default param values of a built-in device (plain units). */
export function defaultParams(device: Parameters<typeof builtinDescriptor>[0]): Record<number, number> {
  const params: Record<number, number> = {};
  for (const p of builtinDescriptor(device).params) params[p.id] = p.default;
  return params;
}

/**
 * A new empty project: master track, tempo 120 and 4/4 at beat 0, default settings.
 * `nextId` generates entity ids; `id` is the project's UUIDv7.
 */
export function createEmptyProject(nextId: () => string, name: string, id: ProjectId): Project {
  const master = makeTrack({ id: nextId(), kind: "Master", name: "Master", color: 0x9a9a9a, order: "a0" });
  const tempoId = nextId();
  const sigId = nextId();
  return {
    id,
    settings: defaultSettings(name),
    tracks: { [master.id]: master },
    // v0.2 tables (contracts-3).
    take_lanes: {},
    comp_regions: {},
    rack_chains: {},
    modulators: {},
    mod_mappings: {},
    clips: {},
    notes: {},
    devices: {},
    sends: {},
    automation_lanes: {},
    automation_points: {},
    tempo_points: { [tempoId]: { id: tempoId, time: 0, bpm: 120, curve: "Step" } },
    time_signatures: { [sigId]: { id: sigId, time: 0, signature: { numerator: 4, denominator: 4 } } },
    warp_markers: {},
    media: {},
    markers: {},
    midi_mappings: {},
    drum_pads: {},
    chat: {},
    pinned_notes: {},
  };
}

/**
 * The demo project:
 * - tracks (in order): "Keys" (MIDI, Synth, 4-bar clip with 16 notes, a volume
 *   automation lane with 3 points, a send to the return), "Bass" (MIDI,
 *   4-bar clip), "Drums" (audio, 8 s stereo 44.1 kHz loop clip), "A Delay" (return, Delay
 *   device), "Master";
 * - tempo 120 at 0, 4/4 at 0, settings name "Demo".
 */
export function createDemoProject(seed = 1): Project {
  const nextId = seededIdFactory(seed);
  const p = createEmptyProject(nextId, "Demo", seededProjectId(seed));
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
    sidechain: null,
    pad: null,
  };
  const keysComp: Device = {
    id: nextId(),
    track: keys.id,
    order: d2,
    name: "Compressor",
    enabled: true,
    kind: { type: "Builtin", device: { type: "Compressor" } },
    params: defaultParams("Compressor"),
    sidechain: null,
    pad: null,
  };
  const bassSynth: Device = {
    id: nextId(),
    track: bass.id,
    order: d1,
    name: "Synth",
    enabled: true,
    kind: { type: "Builtin", device: { type: "Synth" } },
    params: { ...defaultParams("Synth"), 0: 2, 2: 600, 8: -3 },
    sidechain: null,
    pad: null,
  };
  const delay: Device = {
    id: nextId(),
    track: ret.id,
    order: d1,
    name: "Delay",
    enabled: true,
    kind: { type: "Builtin", device: { type: "Delay" } },
    params: { ...defaultParams("Delay"), 4: 100 },
    sidechain: null,
    pad: null,
  };
  for (const d of [synth, keysComp, bassSynth, delay]) p.devices[d.id] = d;

  // Send Keys → return.
  const send = { id: nextId(), from: keys.id, to: ret.id, level: -12, pre_fader: false };
  p.sends[send.id] = send;

  // Keys: 4-bar chord clip (Am - F - C - G), 16 notes.
  const keysClip = makeClip({ id: nextId(), track: keys.id, start: 0, length: 16, name: "Chords" });
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

  // Bass: 4-bar clip starting at bar 5.
  const bassClip = makeClip({ id: nextId(), track: bass.id, start: 16, length: 16, name: "Bassline" });
  p.clips[bassClip.id] = bassClip;
  [33, 33, 29, 29, 36, 36, 31, 31].forEach((pitch, i) => {
    const n: Note = { id: nextId(), clip: bassClip.id, pitch, velocity: 0.9, release_velocity: 0.5, start: i * 2, duration: 1.5, muted: false };
    p.notes[n.id] = n;
  });

  // Drums: audio clip over a fake 8 s stereo file (16 beats at 120 BPM).
  const media: MediaRef = {
    id: nextId(),
    name: "drum-loop-120.wav",
    file: "media/drum-loop-120.wav",
    sample_rate: 44100,
    channels: 2,
    frames: 44100 * 8,
    hash: null,
    location: { type: "Project" },
  };
  p.media[media.id] = media;
  const drumClip = makeClip({
    id: nextId(),
    track: drums.id,
    start: 0,
    length: 16,
    name: "drum-loop-120",
    content: {
      type: "Audio",
      media: media.id,
      gain: 0,
      transpose: 0,
      fade_in: 0,
      fade_out: 0,
      fade_in_curve: { type: "Linear" },
      fade_out_curve: { type: "Linear" },
      reversed: false,
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

/** Set the tempo of the (single) tempo point at beat 0. */
function setTempo(p: Project, bpm: number): void {
  for (const t of Object.values(p.tempo_points)) t.bpm = bpm;
}

/**
 * "Beat sketch": 96 BPM, a "Drum Bus" group track containing two MIDI tracks ("Kick" and
 * "Hats", routed into the group via `output: Default`), each with a 1-bar pattern.
 */
export function createBeatSketchProject(seed = 2): Project {
  const nextId = seededIdFactory(seed);
  const p = createEmptyProject(nextId, "Beat sketch", seededProjectId(seed));
  setTempo(p, 96);
  const master = Object.values(p.tracks)[0]!;
  const [kGroup, kMaster] = keysBetween(null, null, 2) as [string, string];
  master.order = kMaster;
  const [k1, k2] = keysBetween(null, null, 2) as [string, string];
  const group = makeTrack({ id: nextId(), kind: "Group", name: "Drum Bus", color: MOCK_TRACK_COLORS[2]!, order: kGroup });
  const kick = makeTrack({ id: nextId(), kind: "Midi", name: "Kick", color: MOCK_TRACK_COLORS[0]!, order: k1, parent: group.id });
  const hats = makeTrack({ id: nextId(), kind: "Midi", name: "Hats", color: MOCK_TRACK_COLORS[5]!, order: k2, parent: group.id });
  for (const t of [group, kick, hats]) p.tracks[t.id] = t;
  const pattern = (track: string, name: string, pitch: number, step: number) => {
    const clip = makeClip({ id: nextId(), track, start: 0, length: 4, name });
    p.clips[clip.id] = clip;
    for (let i = 0; i * step < 4; i++) {
      const n: Note = { id: nextId(), clip: clip.id, pitch, velocity: 0.9, release_velocity: 0.5, start: i * step, duration: step / 2, muted: false };
      p.notes[n.id] = n;
    }
  };
  pattern(kick.id, "Kick", 36, 1);
  pattern(hats.id, "Hats", 42, 0.5);
  const devices: Array<[Track, "Synth" | "Compressor"]> = [
    [kick, "Synth"],
    [hats, "Synth"],
    [group, "Compressor"],
  ];
  for (const [t, device] of devices) {
    const d: Device = {
      id: nextId(),
      track: t.id,
      order: "a0",
      name: device,
      enabled: true,
      kind: { type: "Builtin", device: { type: device } },
      params: defaultParams(device),
      sidechain: null,
      pad: null,
    };
    p.devices[d.id] = d;
  }
  return p;
}

/** "Ambient idea": 70 BPM, one MIDI "Pad" track with a long 8-bar chord clip. */
export function createAmbientIdeaProject(seed = 3): Project {
  const nextId = seededIdFactory(seed);
  const p = createEmptyProject(nextId, "Ambient idea", seededProjectId(seed));
  setTempo(p, 70);
  const master = Object.values(p.tracks)[0]!;
  const [kPad, kMaster] = keysBetween(null, null, 2) as [string, string];
  master.order = kMaster;
  const pad = makeTrack({ id: nextId(), kind: "Midi", name: "Pad", color: MOCK_TRACK_COLORS[7]!, order: kPad });
  p.tracks[pad.id] = pad;
  const synth: Device = {
    id: nextId(),
    track: pad.id,
    order: "a0",
    name: "Synth",
    enabled: true,
    kind: { type: "Builtin", device: { type: "Synth" } },
    params: { ...defaultParams("Synth"), 0: 3, 4: 1200, 7: 4000 },
    sidechain: null,
    pad: null,
  };
  p.devices[synth.id] = synth;
  const clip = makeClip({ id: nextId(), track: pad.id, start: 0, length: 32, name: "Drift" });
  p.clips[clip.id] = clip;
  const chords = [
    [52, 59, 64, 71],
    [48, 55, 64, 67],
  ];
  chords.forEach((chord, i) => {
    for (const pitch of chord) {
      const n: Note = { id: nextId(), clip: clip.id, pitch, velocity: 0.5, release_velocity: 0.5, start: i * 16, duration: 15.5, muted: false };
      p.notes[n.id] = n;
    }
  });
  return p;
}

/** The projects the mock's engine-side store starts with; the first one is opened on connect. */
export function createDemoProjects(): Project[] {
  return [createDemoProject(), createBeatSketchProject(), createAmbientIdeaProject()];
}
