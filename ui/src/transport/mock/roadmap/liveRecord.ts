/**
 * Mock of arrangement recording with its live view (`MockLiveRecord`, owned by
 * `live-record`), so the web build shows it too. `Recording::SetRecording { true }` starts
 * playback (no count-in) and emits `Started`; while recording, every few playhead steps
 * (~20 Hz) it emits `Progress` with synthetic peaks for each armed audio track (the same
 * deterministic peaks `Media::GetPeaks` later returns for the committed media, so the real
 * clip replaces the live one seamlessly) and a synthetic note on every beat for armed MIDI
 * tracks. `SetRecording { false }` or stopping the transport commits the takes as one undo
 * step (media + audio clip, MIDI clip with the notes), then emits `PeaksReady` and
 * `Stopped`, like the real controller.
 *
 * Takes (`comping`): a loop wrap closes the current pass and starts a new one (new media,
 * new live take). At the end, a track that recorded several passes, or recorded over its
 * main-lane clips or comp regions, gets one take lane per pass (clip + comp region, newest
 * selected); main-lane material in the recorded range first moves onto a new lane (the
 * first take; clips crossing the range edges are split). Otherwise a plain clip, as before.
 */

import type { ClipId, Command, Event, MediaRef, NoteSpec, Project, ReplyValue, TrackId } from "@/generated";
import { compOf, lanesOf, nextTakeName } from "@/features/comping/model";
import { cmd } from "../../cmd";
import { synthesizePeaks } from "../peaks";
import { bpmAt } from "../tempo";

const UNIT: ReplyValue = { type: "Unit" };
export const LIVE_SAMPLE_RATE = 48_000;
export const LIVE_FRAMES_PER_PEAK = 256;
/** Playhead steps (~16 ms) between two `Progress` events (~20 Hz). */
export const PROGRESS_STEPS = 3;
/** Shortest recorded note / MIDI clip (beats), like the controller. */
const MIN_LENGTH = 1 / 64;

/** What the recording simulation needs from `MockTransport`. */
export interface LiveRecordHost {
  project(): Project;
  emit(event: Event): void;
  newId(): string;
  position(): number;
  playing(): boolean;
  armed(): ReadonlyArray<TrackId>;
  /** Start the transport (from the current position). */
  play(): void;
  /** Add `media` and apply `commands` as one "Record" undo step (emits the patch). */
  commit(media: MediaRef[], commands: Command[]): void;
}

interface AudioTake {
  track: TrackId;
  media: MediaRef;
  /** Peaks already sent. */
  sent: number;
}

interface MockNote {
  pitch: number;
  velocity: number;
  start: number;
  end: number | null;
}

/** A finished loop pass. */
interface Pass {
  start: number;
  end: number;
  bpm: number;
  takes: AudioTake[];
  notes: MockNote[];
}

export class MockLiveRecord {
  recording = false;
  private start = 0;
  private bpm = 120;
  private last = 0;
  private steps = 0;
  private takes: AudioTake[] = [];
  private midiTracks: TrackId[] = [];
  private notes: MockNote[] = [];
  /** Finished loop passes (`comping`); the current one is `start`/`takes`/`notes`. */
  private passes: Pass[] = [];
  /** Live take index of the current pass (1-based). */
  private take = 1;
  private audioTracks: Array<{ id: TrackId; name: string }> = [];
  /** MIDI notes not yet sent (start and/or end). */
  private pendingNotes: Array<{ start: number; pitch: number; velocity: number; length: number | null }> = [];

  constructor(private readonly host: LiveRecordHost) {}

  /** `Recording::SetRecording { enabled }`. */
  set(enabled: boolean): ReplyValue {
    if (enabled) this.begin();
    else this.finish();
    return UNIT;
  }

  private begin(): void {
    if (this.recording) return;
    const p = this.host.project();
    const armed = this.host.armed().map((id) => p.tracks[id]).filter((t) => t !== undefined);
    this.start = Math.max(0, this.host.position());
    this.last = this.start;
    this.bpm = bpmAt(p, this.start);
    this.steps = 0;
    this.passes = [];
    this.take = 1;
    this.audioTracks = armed.filter((t) => t.kind === "Audio").map((t) => ({ id: t.id, name: t.name }));
    this.takes = this.newTakes();
    this.midiTracks = armed.filter((t) => t.kind === "Midi").map((t) => t.id);
    this.notes = [];
    this.pendingNotes = [];
    this.recording = true;
    if (!this.host.playing()) this.host.play();
    this.host.emit({ type: "Recording", event: { type: "Started", tracks: armed.map((t) => t.id) } });
  }

  /** New media for the current pass, one per armed audio track. */
  private newTakes(): AudioTake[] {
    const n = this.passes.length * this.audioTracks.length;
    return this.audioTracks.map((t, i) => {
      const id = this.host.newId();
      return {
        track: t.id,
        sent: 0,
        media: {
          id,
          name: `${t.name} Rec ${n + i + 1}.wav`,
          file: `media/rec-mock-${id}.wav`,
          sample_rate: LIVE_SAMPLE_RATE,
          channels: 1,
          frames: 0,
          hash: null,
          location: { type: "Project" } as const,
        },
      };
    });
  }

  /** A loop wrap / locate: close the current pass, start a new one at `pos`. */
  private wrap(pos: number): void {
    // A loop wrap: the pass ends exactly at the loop end, the next starts at the loop start
    // (the engine records sample-accurately; the mock's playhead moves in steps).
    const { loop_enabled, loop_region } = this.host.project().settings;
    if (loop_enabled && pos >= loop_region.start && this.last <= loop_region.end) {
      this.advance(this.last, loop_region.end);
      this.last = loop_region.end;
      const frames = Math.floor((((this.last - this.start) * 60) / this.bpm) * LIVE_SAMPLE_RATE);
      for (const t of this.takes) t.media.frames = Math.max(t.media.frames, frames);
      pos = loop_region.start;
    } else {
      this.advance(this.last, this.last);
    }
    this.progress(true);
    this.passes.push({ start: this.start, end: this.last, bpm: this.bpm, takes: this.takes, notes: this.notes });
    this.start = pos;
    this.last = pos;
    this.bpm = bpmAt(this.host.project(), pos);
    this.take += 1;
    this.takes = this.newTakes();
    this.notes = [];
  }

  /** Every playhead step (after the position advanced). */
  step(): void {
    if (!this.recording) return;
    const pos = this.host.position();
    if (pos < this.last) {
      this.wrap(pos);
      return;
    }
    this.advance(this.last, pos);
    this.last = pos;
    const frames = Math.floor((((pos - this.start) * 60) / this.bpm) * LIVE_SAMPLE_RATE);
    for (const t of this.takes) t.media.frames = Math.max(t.media.frames, frames);
    this.steps += 1;
    if (this.steps % PROGRESS_STEPS === 0) this.progress(false);
  }

  /** Synthetic MIDI: a note on each beat in `(from, to]`, released half a beat later. */
  private advance(from: number, to: number): void {
    if (this.midiTracks.length === 0) return;
    for (let b = Math.floor(from) + 1; b <= to; b++) {
      if (b < this.start) continue;
      const n: MockNote = { pitch: 60 + (b % 4) * 2, velocity: 100, start: b, end: null };
      this.notes.push(n);
      this.pendingNotes.push({ start: b, pitch: n.pitch, velocity: n.velocity, length: null });
    }
    for (const n of this.notes) {
      if (n.end === null && n.start + 0.5 <= to) {
        n.end = n.start + 0.5;
        this.pendingNotes.push({ start: n.start, pitch: n.pitch, velocity: n.velocity, length: 0.5 });
      }
    }
  }

  /** Emit what is new (`all`: include the last partial peak). */
  private progress(all: boolean): void {
    const audio = [];
    for (const t of this.takes) {
      const count = all ? Math.ceil(t.media.frames / LIVE_FRAMES_PER_PEAK) : Math.floor(t.media.frames / LIVE_FRAMES_PER_PEAK);
      if (count <= t.sent) continue;
      const peaks = synthesizePeaks(
        { ...t.media, channels: 1 },
        { media: t.media.id, samples_per_peak: LIVE_FRAMES_PER_PEAK, start_frame: t.sent * LIVE_FRAMES_PER_PEAK, frame_count: (count - t.sent) * LIVE_FRAMES_PER_PEAK },
      );
      audio.push({
        track: t.track,
        take: this.take,
        start: this.start,
        sample_rate: LIVE_SAMPLE_RATE,
        frames_per_peak: LIVE_FRAMES_PER_PEAK,
        first_peak: t.sent,
        min: peaks.min[0]!,
        max: peaks.max[0]!,
      });
      t.sent += peaks.min[0]!.length;
    }
    const midi = this.pendingNotes.flatMap((n) => this.midiTracks.map((track) => ({ track, ...n })));
    this.pendingNotes = [];
    if (audio.length === 0 && midi.length === 0) return;
    this.host.emit({ type: "Recording", event: { type: "Progress", audio, midi } });
  }

  /** Drop the recording without committing (the project was switched). */
  abort(): void {
    this.recording = false;
  }

  /** Stop recording (transport stop or `SetRecording { false }`): commit the takes. */
  finish(): void {
    if (!this.recording) return;
    // The rest of the live view (with the last partial peak), before the real clips.
    this.advance(this.last, this.last);
    this.progress(true);
    this.recording = false;
    const passes: Pass[] = [
      ...this.passes,
      { start: this.start, end: Math.max(this.last, this.start), bpm: this.bpm, takes: this.takes, notes: this.notes },
    ];
    this.passes = [];
    const media: MediaRef[] = [];
    const commands: Command[] = [];
    const clips: ClipId[] = [];
    const project = this.host.project();
    const laneNames = new Map<TrackId, string[]>();
    /** A new lane of `track` (named across the commands of this commit). */
    const newLane = (track: TrackId): string => {
      const names = laneNames.get(track) ?? lanesOf(project, track).map((l) => l.name);
      const name = nextTakeName(names.map((n, i) => ({ id: String(i), track, order: "", name: n, color: null })));
      laneNames.set(track, [...names, name]);
      const id = this.host.newId();
      commands.push(cmd("Take", { type: "CreateLane", id, track, name, before: null }));
      return id;
    };
    const asTake = (track: TrackId, clip: ClipId, start: number, end: number) => {
      const lane = newLane(track);
      commands.push(cmd("Take", { type: "MoveToLane", clips: [clip], lane }));
      commands.push(cmd("Take", { type: "SetComp", id: this.host.newId(), split_id: this.host.newId(), track, lane, start, end }));
    };
    // Audio: one clip per pass and track.
    for (const track of this.audioTracks) {
      const takes = passes
        .map((p) => ({ p, t: p.takes.find((t) => t.track === track.id)! }))
        .filter(({ t }) => t.media.frames > 0);
      if (takes.length === 0) continue;
      const range = ({ p, t }: (typeof takes)[number]): [number, number] => [
        p.start,
        p.start + (t.media.frames / LIVE_SAMPLE_RATE) * (p.bpm / 60),
      ];
      const from = Math.min(...takes.map((x) => range(x)[0]));
      const until = Math.max(...takes.map((x) => range(x)[1]));
      const asTakes = takes.length > 1 || rangeUsed(project, track.id, from, until);
      if (asTakes) this.beginTakes(project, track.id, from, until, commands, newLane);
      for (const x of takes) {
        const id = this.host.newId();
        media.push(x.t.media);
        commands.push(cmd("Clip", { type: "CreateAudio", id, track: track.id, start: x.p.start, media: x.t.media.id }));
        commands.push(cmd("Warp", { type: "SetWarp", clip: id, warp: { enabled: false, mode: "Complex", source_bpm: x.p.bpm } }));
        if (asTakes) asTake(track.id, id, ...range(x));
        clips.push(id);
      }
    }
    // MIDI: one clip per pass with notes, per armed MIDI track.
    const midiPasses = passes.filter((p) => p.notes.length > 0);
    if (midiPasses.length > 0) {
      const spans = midiPasses.map((p): [number, number] => {
        const last = Math.max(p.end, ...p.notes.map((n) => n.end ?? p.end));
        return [p.start, Math.max(p.start + MIN_LENGTH, last)];
      });
      const from = Math.min(...spans.map((s) => s[0]));
      const until = Math.max(...spans.map((s) => s[1]));
      for (const track of this.midiTracks) {
        const asTakes = midiPasses.length > 1 || rangeUsed(project, track, from, until);
        if (asTakes) this.beginTakes(project, track, from, until, commands, newLane);
        midiPasses.forEach((p, i) => {
          const [start, end] = spans[i]!;
          const id = this.host.newId();
          const notes: NoteSpec[] = p.notes.map((n) => ({
            id: this.host.newId(),
            pitch: n.pitch,
            velocity: n.velocity / 127,
            start: n.start - start,
            duration: Math.max(MIN_LENGTH, (n.end ?? p.end) - n.start),
          }));
          commands.push(
            cmd("Clip", { type: "CreateMidi", id, track, start, length: end - start, name: project.tracks[track]?.name ?? null }),
          );
          commands.push(cmd("Note", { type: "Add", clip: id, notes }));
          if (asTakes) asTake(track, id, start, end);
          clips.push(id);
        });
      }
    }
    if (commands.length > 0) this.host.commit(media, commands);
    for (const m of media) this.host.emit({ type: "Media", event: { type: "PeaksReady", media: m.id } });
    this.host.emit({ type: "Recording", event: { type: "Stopped", clips } });
  }

  /**
   * Before takes over `[from, until)`: main-lane clips there move onto a new lane (the first
   * take; edge-crossing clips are split), selected over the range.
   */
  private beginTakes(
    project: Project,
    track: TrackId,
    from: number,
    until: number,
    commands: Command[],
    newLane: (t: TrackId) => string,
  ): void {
    const clips = Object.values(project.clips).filter(
      (c) => c.track === track && c.lane == null && c.start + c.length > from + EPS && c.start < until - EPS,
    );
    if (clips.length === 0) return;
    const inside: ClipId[] = [];
    for (const c of clips) {
      let id = c.id;
      if (c.start < from - EPS) {
        const right = this.host.newId();
        commands.push(cmd("Clip", { type: "Split", id, at: from, new_id: right }));
        id = right;
      }
      if (c.start + c.length > until + EPS) commands.push(cmd("Clip", { type: "Split", id, at: until, new_id: this.host.newId() }));
      inside.push(id);
    }
    const lane = newLane(track);
    commands.push(cmd("Take", { type: "MoveToLane", clips: inside, lane }));
    commands.push(cmd("Take", { type: "SetComp", id: this.host.newId(), split_id: this.host.newId(), track, lane, start: from, end: until }));
  }
}

const EPS = 1e-6;

/** Main-lane clips or comp regions of `track` in `[from, until)`: a recording there becomes takes. */
function rangeUsed(project: Project, track: TrackId, from: number, until: number): boolean {
  const hits = (s: number, e: number) => e > from + EPS && s < until - EPS;
  return (
    Object.values(project.clips).some((c) => c.track === track && c.lane == null && hits(c.start, c.start + c.length)) ||
    compOf(project, track).some((r) => hits(r.start, r.end))
  );
}
