/**
 * Mock of `Capture::*` (v0.3, `capture-midi`, CONTRACTS.md §13.4): the always-on MIDI capture
 * buffer, so the web build and tests show Capture too. `MockTransport.simulateMidiInput`
 * feeds it (every port, armed or not, playing or stopped). Like the controller:
 *
 * - the ring keeps the newest `CAPTURE_MAX_EVENTS` messages of the last `CAPTURE_MAX_SECONDS`;
 *   a play/stop transition starts a new take; a project load clears it;
 * - `Capture` uses the messages that reach the track's input filter, from the latest take with
 *   notes. Played while running: notes keep their song positions, the clip spans the played
 *   bars. Played while stopped: the last phrase (after a 6 s silence) goes at the playhead;
 *   with `adopt_tempo` the tempo is inferred (60..=180 bpm, first note = downbeat, prior
 *   towards 120) and the loop set to the clip. One "Capture" undo step with the clip, its
 *   notes (`deriveId(seed_notes, i)` in (start, pitch) order) and CC / bend / channel-pressure
 *   lanes (step curves); the buffer is then emptied;
 * - `Event::Capture { Changed }` on availability changes only.
 *
 * Mock limitations: the playhead position of a message is the mock's position when it is
 * simulated (no receive-time compensation); bars use the signature at the clip start.
 */

import type {
  BeatRange,
  CaptureCommand,
  CaptureResult,
  CaptureStatus,
  ClipId,
  Command,
  Event,
  ExpressionKind,
  ExpressionLane,
  ExpressionPoint,
  NoteId,
  NoteSpec,
  Project,
  ReplyValue,
  TrackId,
} from "@/generated";
import { deriveId } from "@/features/comping/model";
import { cmd } from "../../cmd";
import { fail } from "../documentReducer";
import { beatsPerBar, bpmAt, signatureAt } from "../tempo";

export const CAPTURE_MAX_SECONDS = 600;
export const CAPTURE_MAX_EVENTS = 65_536;
/** While stopped, a silence (nothing held) this long starts a new phrase. */
export const PHRASE_GAP_SECONDS = 6;
const PRE_ROLL_SECONDS = 0.5;
const MIN_NOTE = 1 / 64;
const BAR_TOLERANCE = 0.25;
const MAX_EXPRESSION_POINTS = 16_384;

/** What the capture simulation needs from `MockTransport`. */
export interface CaptureHost {
  project(): Project;
  emit(event: Event): void;
  newId(): string;
  /** Mock clock, ms. */
  now(): number;
  position(): number;
  playing(): boolean;
  /** Apply `commands` and insert `lanes` as one "Capture" undo step (emits the patch). */
  commit(commands: Command[], lanes: ExpressionLane[]): void;
}

interface Captured {
  port: string;
  data: [number, number, number];
  timeMs: number;
  song: number | null;
  take: number;
}

interface Msg {
  data: [number, number, number];
  at: number;
}

interface DraftNote {
  pitch: number;
  velocity: number;
  start: number;
  duration: number;
}

interface Plan {
  start: number;
  length: number;
  notes: DraftNote[];
  lanes: Array<[ExpressionKind, ExpressionPoint[]]>;
  bpm: number | null;
  setLoop: boolean;
}

const isNoteOn = (d: readonly number[]) => (d[0]! & 0xf0) === 0x90 && d[2]! > 0;
const isNoteOff = (d: readonly number[]) => (d[0]! & 0xf0) === 0x80 || ((d[0]! & 0xf0) === 0x90 && d[2] === 0);

export class MockCapture {
  private ring: Captured[] = [];
  private take = 0;
  private wasPlaying = false;
  private announced = false;

  constructor(private readonly host: CaptureHost) {}

  /** One incoming MIDI message (`simulateMidiInput`). */
  input(port: string, data: [number, number, number]): void {
    if (!(data[0] >= 0x80 && data[0] < 0xf0)) return;
    this.sync();
    const timeMs = this.host.now();
    this.ring.push({ port, data: [...data], timeMs, song: this.host.playing() ? this.host.position() : null, take: this.take });
    const oldest = timeMs - CAPTURE_MAX_SECONDS * 1000;
    let drop = 0;
    while (drop < this.ring.length && (this.ring.length - drop > CAPTURE_MAX_EVENTS || this.ring[drop]!.timeMs < oldest)) drop++;
    if (drop) this.ring.splice(0, drop);
    this.announce();
  }

  /** The project changed: forget everything. */
  projectChanged(): void {
    this.ring = [];
    this.announce();
  }

  /** Play/stop transitions (called on every transport change). */
  sync(): void {
    const playing = this.host.playing();
    if (playing !== this.wasPlaying) {
      this.wasPlaying = playing;
      this.take++;
    }
  }

  status(): CaptureStatus {
    const notes = this.ring.filter((m) => isNoteOn(m.data)).length;
    const first = this.ring[0];
    const last = this.ring.at(-1);
    return { available: notes > 0, notes, seconds: first && last ? (last.timeMs - first.timeMs) / 1000 : 0 };
  }

  command(c: CaptureCommand): ReplyValue {
    this.sync();
    switch (c.type) {
      case "Status":
        return { type: "CaptureStatus", status: this.status() };
      case "Clear":
        this.ring = [];
        this.announce();
        return { type: "Unit" };
      case "Capture": {
        const capture = this.capture(c.track, c.clip, c.seed_notes, c.adopt_tempo);
        this.ring = [];
        this.announce();
        return { type: "Captured", capture };
      }
    }
  }

  private announce(): void {
    const status = this.status();
    if (status.available === this.announced) return;
    this.announced = status.available;
    this.host.emit({ type: "Capture", event: { type: "Changed", status } });
  }

  private capture(trackId: TrackId, clip: ClipId, seed: NoteId, adoptTempo: boolean): CaptureResult {
    const project = this.host.project();
    const track = project.tracks[trackId] ?? fail("NotFound", `track ${trackId}`);
    if (track.kind !== "Midi") fail("InvalidArgument", "Capture needs a MIDI track");
    if (project.clips[clip]) fail("InvalidArgument", `clip ${clip} already exists`);
    const input = track.input;
    const heard = this.ring.filter(
      (m) => input.type !== "Midi" || ((input.port === null || input.port === m.port) && (input.channel === null || input.channel === (m.data[0] & 0x0f))),
    );
    const take = [...heard].reverse().find((m) => isNoteOn(m.data))?.take;
    if (take === undefined) fail("InvalidState", "nothing was played to capture");
    const msgs = heard.filter((m) => m.take === take);
    const plan = msgs.some((m) => m.song !== null) ? this.planPlaying(project, msgs) : this.planStopped(project, msgs, adoptTempo);
    if (!plan) fail("InvalidState", "nothing was played to capture");

    const commands: Command[] = [];
    if (plan.bpm !== null) commands.push(cmd("Transport", { type: "SetTempo", bpm: plan.bpm }));
    if (plan.setLoop) {
      const region: BeatRange = { start: plan.start, end: plan.start + plan.length };
      commands.push(cmd("Transport", { type: "SetLoopRegion", region }));
      commands.push(cmd("Transport", { type: "SetLoopEnabled", enabled: true }));
    }
    commands.push(cmd("Clip", { type: "CreateMidi", id: clip, track: trackId, start: plan.start, length: plan.length, name: track.name }));
    const notes: NoteSpec[] = plan.notes.map((n, i) => ({
      id: deriveId(seed, i),
      pitch: n.pitch,
      velocity: n.velocity,
      start: Math.max(0, n.start),
      duration: Math.max(MIN_NOTE, n.duration),
    }));
    commands.push(cmd("Note", { type: "Add", clip, notes }));
    const lanes: ExpressionLane[] = plan.lanes.map(([kind, points]) => ({ id: this.host.newId(), clip, kind, points }));
    this.host.commit(commands, lanes);
    return { clip, start: plan.start, length: plan.length, notes: notes.length, bpm: plan.bpm };
  }

  private planPlaying(project: Project, msgs: Captured[]): Plan | null {
    const s = project.settings;
    const loopEnd = s.loop_enabled && s.loop_region.end > s.loop_region.start ? s.loop_region.end : null;
    const seq: Msg[] = msgs.filter((m) => m.song !== null).map((m) => ({ data: m.data, at: m.song! }));
    const last = seq.at(-1)?.at;
    if (last === undefined) return null;
    const end = this.host.playing() ? Math.max(last, this.host.position()) : last;
    const notes = pairNotes(seq, end, loopEnd);
    const first = notes[0];
    if (!first) return null;
    const bar = beatsPerBar(signatureAt(project, first.start));
    const start = Math.floor(first.start / bar + 1e-9) * bar;
    const noteEnd = Math.max(...notes.map((n) => n.start + Math.max(n.duration, MIN_NOTE)));
    const length = Math.max(1, Math.ceil((noteEnd - start) / bar - 1e-9)) * bar;
    return {
      start,
      length,
      notes: notes.map((n) => ({ ...n, start: n.start - start })),
      lanes: expressionLanes(seq, (b) => b - start),
      bpm: null,
      setLoop: false,
    };
  }

  private planStopped(project: Project, msgs: Captured[], adoptTempo: boolean): Plan | null {
    const all: Msg[] = msgs.map((m) => ({ data: m.data, at: m.timeMs / 1000 }));
    const from = lastPhraseStart(all);
    if (from === null) return null;
    const phrase = all.slice(from);
    const t0 = phrase.find((m) => isNoteOn(m.data))!.at;
    const rel = phrase.map((m) => ({ data: m.data, at: m.at - t0 }));
    const end = Math.max(this.host.now() / 1000 - t0, rel.at(-1)!.at);
    const notesS = pairNotes(rel, end, null);
    const at = Math.max(0, this.host.position());
    const bpm = adoptTempo ? inferBpm(notesS.map((n) => n.start)) : null;
    const spb = 60 / (bpm ?? bpmAt(project, at));
    const toBeats = (t: number) => t / spb;
    const notes = notesS.map((n) => ({ ...n, start: toBeats(n.start), duration: toBeats(n.duration) }));
    const noteEnd = Math.max(0, ...notes.map((n) => n.start + Math.max(n.duration, MIN_NOTE)));
    const bar = beatsPerBar(signatureAt(project, at));
    const length = Math.max(1, Math.ceil(Math.max(0, noteEnd - BAR_TOLERANCE) / bar - 1e-9)) * bar;
    return { start: at, length, notes, lanes: expressionLanes(rel, toBeats), bpm, setLoop: adoptTempo };
  }
}

function lastPhraseStart(msgs: Msg[]): number | null {
  const held: string[] = [];
  let quietSince: number | null = null;
  let start: number | null = null;
  msgs.forEach((m, i) => {
    const key = `${m.data[0] & 0x0f}:${m.data[1]}`;
    if (isNoteOn(m.data)) {
      if (held.length === 0 && (quietSince === null || m.at - quietSince >= PHRASE_GAP_SECONDS)) start = i;
      held.push(key);
    } else if (isNoteOff(m.data)) {
      const j = held.indexOf(key);
      if (j >= 0) {
        held.splice(j, 1);
        if (held.length === 0) quietSince = m.at;
      }
    }
  });
  if (start === null) return null;
  const t = msgs[start]!.at - PRE_ROLL_SECONDS;
  let from = start;
  while (from > 0 && msgs[from - 1]!.at >= t) from--;
  return from;
}

function pairNotes(msgs: Msg[], end: number, loopEnd: number | null): DraftNote[] {
  const held: Array<{ ch: number; key: number; t0: number; v: number }> = [];
  const out: DraftNote[] = [];
  const close = (t0: number, t1: number) => Math.max(0, (t1 < t0 ? (loopEnd ?? t0) : t1) - t0);
  for (const m of msgs) {
    const on = isNoteOn(m.data);
    if (!on && !isNoteOff(m.data)) continue;
    const ch = m.data[0] & 0x0f;
    const key = m.data[1] & 0x7f;
    const i = held.findIndex((h) => h.ch === ch && h.key === key);
    if (i >= 0) {
      const h = held.splice(i, 1)[0]!;
      out.push({ pitch: key, velocity: h.v, start: h.t0, duration: close(h.t0, m.at) });
    }
    if (on) held.push({ ch, key, t0: m.at, v: (m.data[2] & 0x7f) / 127 });
  }
  for (const h of held) out.push({ pitch: h.key, velocity: h.v, start: h.t0, duration: close(h.t0, end) });
  return out.sort((a, b) => a.start - b.start || a.pitch - b.pitch);
}

function expressionOf(d: readonly number[]): [ExpressionKind, number] | null {
  switch (d[0]! & 0xf0) {
    case 0xb0:
      return d[1]! <= 119 ? [{ type: "Cc", controller: d[1]! }, (d[2]! & 0x7f) / 127] : null;
    case 0xd0:
      return [{ type: "ChannelPressure" }, (d[1]! & 0x7f) / 127];
    case 0xe0: {
      const raw = (d[1]! & 0x7f) | ((d[2]! & 0x7f) << 7);
      return [{ type: "PitchBend" }, Math.max(-1, Math.min(1, (raw - 8192) / 8191))];
    }
    default:
      return null;
  }
}

function expressionLanes(msgs: Msg[], toBeats: (t: number) => number): Array<[ExpressionKind, ExpressionPoint[]]> {
  const lanes = new Map<string, [ExpressionKind, Array<[number, number]>]>();
  for (const m of msgs) {
    const e = expressionOf(m.data);
    if (!e) continue;
    const key = JSON.stringify(e[0]);
    if (!lanes.has(key)) lanes.set(key, [e[0], []]);
    lanes.get(key)![1].push([Math.max(0, toBeats(m.at)), e[1]]);
  }
  return [...lanes.values()].map(([kind, pts]) => {
    pts.sort((a, b) => a[0] - b[0]);
    const curve: ExpressionPoint[] = [];
    for (const [time, value] of pts) {
      const last = curve.at(-1);
      if (last && last.time === time) {
        last.value = value;
        continue;
      }
      if (last && last.value === value) continue;
      curve.push({ time, value, curve: { type: "Step" } });
    }
    return [kind, curve.slice(-MAX_EXPRESSION_POINTS)];
  });
}

// ─── Tempo inference (port of `ether-controller/src/capture/tempo.rs`) ──────────────────

const LEVELS: Array<[number, number]> = [
  [1, 0],
  [0.5, 0.15],
  [1 / 3, 0.3],
  [0.25, 0.35],
];

function place(t: number, bpm: number): [number, number] {
  const spb = 60 / bpm;
  const x = t / spb;
  let best: [number, number] = [Math.round(x), 1];
  for (const [grid, levelCost] of LEVELS) {
    const at = Math.round(x / grid) * grid;
    const dev = (Math.abs(x - at) * spb) / 0.04;
    const cost = Math.min(levelCost + dev * dev, 1);
    if (cost < best[1]) best = [at, cost];
  }
  return best;
}

function score(rel: number[], bpm: number): number {
  const fit = rel.reduce((s, t) => s + place(t, bpm)[1], 0) / rel.length;
  const spb = 60 / bpm;
  const beats = Math.floor(rel.at(-1)! / spb + 0.125) + 1;
  const sounded = new Array<boolean>(beats).fill(false);
  for (const t of rel) {
    const x = t / spb;
    const k = Math.round(x);
    if (Math.abs(x - k) <= 0.125 && k < beats) sounded[k] = true;
  }
  const empty = sounded.filter((s) => !s).length / beats;
  const octaves = Math.log2(bpm / 120);
  return fit + 0.3 * empty + 0.4 * octaves * octaves;
}

/** Onsets (seconds) → tempo in 60..=180 bpm, or `null` with fewer than two onsets. */
export function inferBpm(onsets: number[]): number | null {
  const sorted = onsets.filter(Number.isFinite).sort((a, b) => a - b);
  const distinct: number[] = [];
  for (const t of sorted) if (distinct.length === 0 || t - distinct.at(-1)! > 0.04) distinct.push(t);
  if (distinct.length < 2) return null;
  const rel = distinct.slice(0, 512).map((t) => t - distinct[0]!);
  let best: [number, number] | null = null;
  for (let i = 0; i <= 1200; i++) {
    const bpm = 60 + i * 0.1;
    const cost = score(rel, bpm);
    if (!best || cost < best[1] - 1e-9 || (cost <= best[1] + 1e-9 && Math.abs(bpm - 120) < Math.abs(best[0] - 120) - 0.05)) {
      best = [bpm, cost];
    }
  }
  const bpm = best![0];
  const ks = rel.map((t) => place(t, bpm)[0]);
  const mk = ks.reduce((a, b) => a + b, 0) / ks.length;
  const mt = rel.reduce((a, b) => a + b, 0) / rel.length;
  let cov = 0;
  let v = 0;
  ks.forEach((k, i) => {
    cov += (k - mk) * (rel[i]! - mt);
    v += (k - mk) * (k - mk);
  });
  let fitted = v > 0 && cov > 0 ? 60 / (cov / v) : bpm;
  if (Math.abs(fitted - bpm) > 0.1) fitted = bpm;
  return Math.min(180, Math.max(60, Math.round(fitted * 100) / 100));
}
