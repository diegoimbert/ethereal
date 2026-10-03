/**
 * Mock of `AudioToMidi::*` (v0.3, contracts-4). Owned by `audio-to-midi`. Like the Rust
 * controller (`ether-controller/src/audio_to_midi`): `Start` checks the clip (audio, media
 * present), the new ids and the options, and replies `Unit`; the job then reports
 * `Progress` every [`STEP_MS`] and applies one undo step (a MIDI track right below the
 * source, a clip at the source clip's position and length, the notes, an optional Poly
 * Synth / Drum Rack) before `Done`. One job at a time (`InvalidState`); `Cancel` emits
 * `Cancelled`; the job is cancelled when its clip goes away.
 *
 * There is no audio in the mock: the "detected" notes are a deterministic pattern per mode
 * (a melody line, block chords, a kick/snare/hat beat) across the clip. Needs the mock host
 * (`MockTransport` passes it; without one every command fails `Unsupported`).
 */

import type {
  AudioToMidiCommand,
  AudioToMidiMode,
  AudioToMidiOptions,
  Clip,
  Command,
  NoteSpec,
  Project,
  ReplyValue,
  Track,
  TrackId,
} from "@/generated";
import { cmd } from "../../cmd";
import { fail } from "../documentReducer";
import type { MockHost } from "./host";

const UNIT: ReplyValue = { type: "Unit" };
/** Time between progress steps, and the steps before the result. */
export const STEP_MS = 80;
const STEPS = 4;

interface Job {
  job: string;
  clip: string;
  timer: ReturnType<typeof setTimeout> | null;
  step: number;
}

/** The running job of each mock host (one at a time, like the engine). */
const jobs = new WeakMap<MockHost, Job>();

function checkOptions(o: AudioToMidiOptions): void {
  if (!(Number.isFinite(o.sensitivity) && o.sensitivity >= 0 && o.sensitivity <= 1)) fail("InvalidArgument", "sensitivity must be in 0..=1");
  if (!(Number.isFinite(o.min_duration) && o.min_duration >= 0 && o.min_duration <= 10)) fail("InvalidArgument", "min_duration must be in 0..=10 seconds");
  if (o.min_pitch > o.max_pitch || o.max_pitch > 127) fail("InvalidArgument", "the pitch range must satisfy min <= max <= 127");
  if ([o.kick_key, o.snare_key, o.hihat_key].some((k) => k > 127)) fail("InvalidArgument", "drum keys must be MIDI keys (0..=127)");
}

/** The track right after `t` among its siblings ("below"). */
function nextSibling(p: Project, t: Track): TrackId | null {
  const sibs = Object.values(p.tracks)
    .filter((x) => x.parent === t.parent)
    .sort((a, b) => (a.order < b.order ? -1 : a.order > b.order ? 1 : a.id < b.id ? -1 : 1));
  const i = sibs.findIndex((x) => x.id === t.id);
  return sibs[i + 1]?.id ?? null;
}

/** `(beat, beats, key, velocity)` notes the mock "detects" in a clip `length` beats long. */
function pattern(mode: AudioToMidiMode, o: AudioToMidiOptions, length: number): Array<[number, number, number, number]> {
  const out: Array<[number, number, number, number]> = [];
  const clampKey = (k: number) => Math.min(o.max_pitch, Math.max(o.min_pitch, k));
  switch (mode) {
    case "Melody": {
      const line = [60, 62, 64, 67, 69, 67, 64, 62];
      for (let b = 0, i = 0; b < length; b += 0.5, i++) out.push([b, 0.45, clampKey(line[i % line.length]!), 0.8]);
      break;
    }
    case "Harmony": {
      const chords = [
        [48, 60, 64, 67],
        [45, 57, 60, 64],
        [41, 57, 60, 65],
        [43, 55, 59, 62],
      ];
      for (let b = 0, i = 0; b < length; b += 2, i++)
        for (const k of chords[i % chords.length]!) out.push([b, Math.min(1.9, length - b), clampKey(k), 0.7]);
      break;
    }
    case "Drums":
      for (let b = 0; b < length; b += 0.5) {
        const beat = Math.round(b * 2);
        if (beat % 4 === 0) out.push([b, 0.25, o.kick_key, 1]);
        if (beat % 4 === 2) out.push([b, 0.25, o.snare_key, 0.9]);
        out.push([b, 0.25, o.hihat_key, beat % 2 === 0 ? 0.7 : 0.45]);
      }
      break;
  }
  return out
    .map(([s, d, k, v]): [number, number, number, number] => [s, Math.min(d, length - s), k, v])
    .filter(([, d]) => d > 1e-6)
    .sort((a, b) => a[0] - b[0] || a[2] - b[2]);
}

function finish(host: MockHost, c: Extract<AudioToMidiCommand, { type: "Start" }>): number {
  const p = host.project();
  const clip: Clip = p.clips[c.clip] ?? fail("NotFound", `clip ${c.clip} not found`);
  const source = p.tracks[clip.track] ?? fail("NotFound", `track ${clip.track} not found`);
  const name = clip.name.trim() || source.name;
  const notes: NoteSpec[] = pattern(c.mode, c.options, clip.length).map(([start, duration, pitch, velocity]) => ({
    id: host.newId(),
    pitch,
    velocity,
    start,
    duration,
  }));
  const commands: Command[] = [
    cmd("Track", {
      type: "Create",
      id: c.track,
      kind: "Midi",
      name: `${name} MIDI`,
      color: source.color,
      parent: source.parent,
      before: nextSibling(p, source),
    }),
    cmd("Clip", { type: "CreateMidi", id: c.new_clip, track: c.track, start: clip.start, length: clip.length, name }),
  ];
  if (notes.length) commands.push(cmd("Note", { type: "Add", clip: c.new_clip, notes }));
  if (c.instrument)
    commands.push(
      cmd("Device", {
        type: "Insert",
        id: c.instrument,
        track: c.track,
        device: { type: "Builtin", device: c.mode === "Drums" ? { type: "DrumRack" } : { type: "PolySynth" } },
        before: null,
      }),
    );
  host.applyDocument(commands, "Convert to MIDI");
  return notes.length;
}

function stepJob(host: MockHost, job: Job, c: Extract<AudioToMidiCommand, { type: "Start" }>): void {
  job.timer = null;
  if (jobs.get(host) !== job) return;
  const clip = host.project().clips[job.clip];
  if (!clip || clip.content.type !== "Audio") {
    jobs.delete(host);
    host.emit({ type: "AudioToMidi", event: { type: "Cancelled", job: job.job } });
    return;
  }
  job.step++;
  if (job.step < STEPS) {
    host.emit({ type: "AudioToMidi", event: { type: "Progress", job: job.job, progress: job.step / STEPS } });
    job.timer = setTimeout(() => stepJob(host, job, c), STEP_MS);
    return;
  }
  jobs.delete(host);
  try {
    const notes = finish(host, c);
    host.emit({ type: "AudioToMidi", event: { type: "Progress", job: job.job, progress: 1 } });
    host.emit({ type: "AudioToMidi", event: { type: "Done", job: job.job, track: c.track, clip: c.new_clip, notes } });
  } catch (err) {
    const message = err && typeof err === "object" && "message" in err ? String(err.message) : String(err);
    host.emit({ type: "AudioToMidi", event: { type: "Failed", job: job.job, message: `Convert to MIDI: ${message}` } });
  }
}

export function audioToMidiCommand(c: AudioToMidiCommand, host?: MockHost): ReplyValue {
  if (!host) return fail("Unsupported", `AudioToMidi::${c.type} needs the mock host (audio-to-midi)`);
  const running = jobs.get(host);
  if (c.type === "Cancel") {
    if (running?.job === c.job) {
      if (running.timer) clearTimeout(running.timer);
      jobs.delete(host);
      host.emit({ type: "AudioToMidi", event: { type: "Cancelled", job: c.job } });
    }
    return UNIT;
  }
  if (!c.job) fail("InvalidArgument", "empty job id");
  if (running) fail("InvalidState", "an audio to MIDI conversion is already running");
  checkOptions(c.options);
  const p = host.project();
  const clip = p.clips[c.clip] ?? fail("NotFound", `clip ${c.clip} not found`);
  if (clip.content.type !== "Audio") fail("InvalidArgument", `clip ${c.clip} is not an audio clip`);
  if (!p.media[clip.content.media]) fail("NotFound", `media ${clip.content.media} not found`);
  if (p.tracks[c.track]) fail("InvalidArgument", `track ${c.track} already exists`);
  if (p.clips[c.new_clip]) fail("InvalidArgument", `clip ${c.new_clip} already exists`);
  if (c.instrument && p.devices[c.instrument]) fail("InvalidArgument", `device ${c.instrument} already exists`);
  const job: Job = { job: c.job, clip: c.clip, timer: null, step: 0 };
  jobs.set(host, job);
  job.timer = setTimeout(() => stepJob(host, job, c), STEP_MS);
  return UNIT;
}
