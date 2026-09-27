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
 */

import type { ClipId, Command, Event, MediaRef, NoteSpec, Project, ReplyValue, TrackId } from "@/generated";
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

export class MockLiveRecord {
  recording = false;
  private start = 0;
  private bpm = 120;
  private last = 0;
  private steps = 0;
  private takes: AudioTake[] = [];
  private midiTracks: TrackId[] = [];
  private notes: MockNote[] = [];
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
    this.takes = armed
      .filter((t) => t.kind === "Audio")
      .map((t, i) => ({
        track: t.id,
        sent: 0,
        media: {
          id: this.host.newId(),
          name: `${t.name} Rec ${i + 1}.wav`,
          file: "",
          sample_rate: LIVE_SAMPLE_RATE,
          channels: 1,
          frames: 0,
          hash: null,
        },
      }));
    for (const t of this.takes) t.media.file = `media/rec-mock-${t.media.id}.wav`;
    this.midiTracks = armed.filter((t) => t.kind === "Midi").map((t) => t.id);
    this.notes = [];
    this.pendingNotes = [];
    this.recording = true;
    if (!this.host.playing()) this.host.play();
    this.host.emit({ type: "Recording", event: { type: "Started", tracks: armed.map((t) => t.id) } });
  }

  /** Every playhead step (after the position advanced). */
  step(): void {
    if (!this.recording) return;
    const pos = this.host.position();
    // A loop wrap / locate: the mock keeps one take and stops growing it.
    if (pos < this.last) {
      this.last = pos;
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
        take: 1,
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
    const end = Math.max(this.last, this.start);
    const media: MediaRef[] = [];
    const commands: Command[] = [];
    const clips: ClipId[] = [];
    for (const t of this.takes) {
      if (t.media.frames <= 0) continue;
      const id = this.host.newId();
      media.push(t.media);
      commands.push(cmd("Clip", { type: "CreateAudio", id, track: t.track, start: this.start, media: t.media.id }));
      commands.push(cmd("Warp", { type: "SetWarp", clip: id, warp: { enabled: false, mode: "Complex", source_bpm: this.bpm } }));
      clips.push(id);
    }
    if (this.notes.length > 0) {
      const last = Math.max(end, ...this.notes.map((n) => (n.end ?? end)));
      for (const track of this.midiTracks) {
        const id = this.host.newId();
        const notes: NoteSpec[] = this.notes.map((n) => ({
          id: this.host.newId(),
          pitch: n.pitch,
          velocity: n.velocity / 127,
          start: n.start - this.start,
          duration: Math.max(MIN_LENGTH, (n.end ?? end) - n.start),
        }));
        commands.push(cmd("Clip", { type: "CreateMidi", id, track, start: this.start, length: Math.max(MIN_LENGTH, last - this.start), name: this.host.project().tracks[track]?.name ?? null }));
        commands.push(cmd("Note", { type: "Add", clip: id, notes }));
        clips.push(id);
      }
    }
    if (commands.length > 0) this.host.commit(media, commands);
    for (const m of media) this.host.emit({ type: "Media", event: { type: "PeaksReady", media: m.id } });
    this.host.emit({ type: "Recording", event: { type: "Stopped", clips } });
  }
}
