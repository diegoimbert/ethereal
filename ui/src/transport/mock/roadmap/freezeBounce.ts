/**
 * Mock of `Freeze::*` (v0.2, `MockFreeze`). Owned by `freeze-bounce`. Render jobs (`Freeze`,
 * `Bounce`, audio `Consolidate`) reply `RenderStarted`, then each playhead step
 * (`MockTransport.tick()` with manual timers) advances one phase: Progress 0.5, then the
 * document edit (one undo step: the render's media + the edit), Progress 1.0 and Done, like
 * the Rust controller (`ether-controller/src/freeze`). The media is a silent placeholder
 * (no audio in the mock); ids the engine derives from seeds (`derive_id`) are fresh mock ids
 * here. `Unfreeze`, `Flatten` and MIDI-only `Consolidate` are instantaneous.
 */

import type {
  Beats,
  BounceTarget,
  Clip,
  ClipId,
  Command,
  FreezeCommand,
  MediaId,
  MediaRef,
  Project,
  ReplyValue,
  Track,
  TrackId,
} from "@/generated";
import { cmd } from "../../cmd";
import { fail } from "../documentReducer";
import { beatsToSeconds } from "../tempo";
import type { Tx } from "../tx";
import type { MockHost } from "./host";

const UNIT: ReplyValue = { type: "Unit" };
/** Sample rate of the placeholder renders. */
const MOCK_RATE = 48_000;
/** A freeze renders this much tail after the last clip (the engine renders until silence). */
const MOCK_TAIL_SECONDS = 1;

/** `MockHost` plus a raw transaction (media and `Track.freeze` have no document command). */
export interface FreezeHost extends MockHost {
  /**
   * Run `edit` as one undo step `label` (emits the patch): write through `tx`, or apply
   * document commands with `apply`.
   */
  transact(label: string, edit: (tx: Tx, apply: (c: Command) => void) => void): void;
}

type Finish =
  | { type: "Freeze"; track: TrackId }
  | { type: "Bounce"; track: TrackId; start: Beats; target: BounceTarget }
  | { type: "Consolidate"; tracks: TrackId[]; start: Beats; end: Beats };

interface Job {
  job: string;
  phase: number;
  media: MediaRef[];
  finish: Finish;
}

const clipEnd = (c: Clip) => c.start + c.length;

function contentTrack(p: Project, id: TrackId): Track {
  const t = p.tracks[id] ?? fail("NotFound", `track ${id} not found`);
  if (t.kind !== "Audio" && t.kind !== "Midi") fail("InvalidArgument", "only audio and MIDI tracks can be rendered");
  return t;
}

function notFrozen(t: Track): void {
  if (t.freeze) fail("InvalidState", `track "${t.name}" is frozen: unfreeze it first`);
}

function checkRange(start: Beats, end: Beats): void {
  if (!(Number.isFinite(start) && Number.isFinite(end) && start >= 0)) fail("InvalidArgument", "invalid range");
  if (!(end > start + 1e-9)) fail("InvalidArgument", "the range is empty");
}

function mainClipsIn(p: Project, track: TrackId, start: Beats, end: Beats): Clip[] {
  return Object.values(p.clips).filter((c) => c.track === track && !c.lane && c.start < end && clipEnd(c) > start);
}

export class MockFreeze {
  private job: Job | null = null;

  constructor(private readonly host: FreezeHost) {}

  private media(id: MediaId, name: string, seconds: number): MediaRef {
    return {
      id,
      name: `${name}.wav`,
      file: `media/${id}-${name.replace(/[^A-Za-z0-9._-]/g, "_")}.wav`,
      sample_rate: MOCK_RATE,
      channels: 2,
      frames: Math.max(1, Math.round(seconds * MOCK_RATE)),
      hash: null,
      location: { type: "Project" },
    } as MediaRef;
  }

  private start(job: string, media: MediaRef[], finish: Finish): ReplyValue {
    this.job = { job, phase: 0, media, finish };
    return { type: "RenderStarted", job };
  }

  private checkCanStart(job: string): void {
    if (!job) fail("InvalidArgument", "empty job id");
    if (this.job) fail("InvalidState", "a freeze or bounce is already running");
  }

  command(c: FreezeCommand): ReplyValue {
    const p = this.host.project();
    switch (c.type) {
      case "Freeze": {
        this.checkCanStart(c.job);
        const t = contentTrack(p, c.track);
        notFrozen(t);
        if (p.media[c.media]) fail("InvalidArgument", `media ${c.media} already exists`);
        const end = Math.max(0, ...Object.values(p.clips).filter((x) => x.track === c.track).map(clipEnd));
        if (end <= 1e-9) fail("InvalidArgument", `track "${t.name}" has no clips to freeze`);
        const seconds = beatsToSeconds(p, end) + MOCK_TAIL_SECONDS;
        return this.start(c.job, [this.media(c.media, `${t.name} Freeze`, seconds)], { type: "Freeze", track: c.track });
      }
      case "Unfreeze": {
        const t = contentTrack(p, c.track);
        if (!t.freeze) return UNIT;
        this.host.transact("Unfreeze", (tx) => {
          const { freeze: _, ...rest } = tx.get("Track", c.track)!;
          tx.upsert("Track", rest as Track);
        });
        return UNIT;
      }
      case "Flatten": {
        const t = contentTrack(p, c.track);
        const f = t.freeze ?? fail("InvalidState", `track "${t.name}" is not frozen`);
        if (p.clips[c.clip]) fail("InvalidArgument", `clip ${c.clip} already exists`);
        this.host.transact("Flatten", (tx, apply) => {
          let target = c.track;
          if (t.kind === "Midi") {
            apply(cmd("Track", { type: "Create", id: c.new_track, kind: "Audio", name: t.name, color: t.color, parent: t.parent, before: t.id }));
            const nt = tx.get("Track", c.new_track)!;
            tx.upsert("Track", { ...nt, mixer: { ...t.mixer }, output: t.output, ...(t.vca ? { vca: t.vca } : {}) });
            const sends = Object.values(tx.project.sends)
              .filter((s) => s.from === t.id)
              .sort((a, b) => (a.id < b.id ? -1 : 1));
            for (const s of sends)
              apply(cmd("Mixer", { type: "CreateSend", id: this.host.newId(), from: c.new_track, to: s.to, level: s.level, pre_fader: s.pre_fader }));
            target = c.new_track;
          } else {
            const { freeze: _, ...rest } = tx.get("Track", c.track)!;
            tx.upsert("Track", rest as Track);
            const clips = Object.values(tx.project.clips).filter((x) => x.track === c.track).map((x) => x.id);
            if (clips.length) apply(cmd("Clip", { type: "Delete", ids: clips }));
            for (const d of Object.values(tx.project.devices).filter((x) => x.track === c.track && !x.pad && !x.chain))
              if (tx.get("Device", d.id)) apply(cmd("Device", { type: "Remove", id: d.id }));
          }
          apply(cmd("Clip", { type: "CreateAudio", id: c.clip, track: target, start: 0, media: f.media }));
          if (t.kind === "Midi") apply(cmd("Track", { type: "Delete", id: t.id }));
        });
        return UNIT;
      }
      case "Bounce": {
        this.checkCanStart(c.job);
        const t = contentTrack(p, c.track);
        checkRange(c.start, c.end);
        if (p.media[c.media]) fail("InvalidArgument", `media ${c.media} already exists`);
        if (!c.include_chain && t.kind === "Midi")
          fail("InvalidArgument", "a MIDI track has no audio before its devices: bounce it with its chain");
        if (t.freeze && (!c.include_chain || c.target.type === "InPlace")) notFrozen(t);
        if (c.target.type === "InPlace" && (t.kind !== "Audio" || c.include_chain))
          fail("InvalidArgument", "bounce in place is for audio tracks, without the device chain");
        if (c.target.type === "NewTrack" && p.tracks[c.target.track]) fail("InvalidArgument", `track ${c.target.track} already exists`);
        if (p.clips[c.target.clip]) fail("InvalidArgument", `clip ${c.target.clip} already exists`);
        const seconds = beatsToSeconds(p, c.end) - beatsToSeconds(p, c.start);
        return this.start(c.job, [this.media(c.media, `${t.name} Bounce`, seconds)], {
          type: "Bounce",
          track: c.track,
          start: c.start,
          target: c.target,
        });
      }
      case "Consolidate": {
        checkRange(c.start, c.end);
        if (c.tracks.length === 0) fail("InvalidArgument", "no tracks to consolidate");
        const tracks = [...new Set(c.tracks)];
        const media: MediaRef[] = [];
        for (const id of tracks) {
          const t = contentTrack(p, id);
          notFrozen(t);
          if (t.kind === "Audio" && mainClipsIn(p, id, c.start, c.end).length)
            media.push(this.media(this.host.newId(), `${t.name} Consolidated`, beatsToSeconds(p, c.end) - beatsToSeconds(p, c.start)));
        }
        const finish: Finish = { type: "Consolidate", tracks, start: c.start, end: c.end };
        if (media.length === 0) {
          this.host.transact("Consolidate", (tx, apply) => this.consolidate(tx, apply, finish, []));
          return UNIT;
        }
        this.checkCanStart(c.job);
        return this.start(c.job, media, finish);
      }
      case "Cancel":
        if (this.job?.job === c.job) {
          this.job = null;
          this.host.emit({ type: "Freeze", event: { type: "Cancelled", job: c.job } });
        }
        return UNIT;
    }
  }

  private consolidate(tx: Tx, apply: (c: Command) => void, f: Extract<Finish, { type: "Consolidate" }>, media: MediaRef[]): void {
    let m = 0;
    for (const id of f.tracks) {
      const t = tx.get("Track", id);
      if (!t) continue;
      const clips = mainClipsIn(tx.project, id, f.start, f.end);
      if (!clips.length) continue;
      const clip: ClipId = this.host.newId();
      if (t.kind === "Midi") {
        const notes = clips
          .filter((x) => !x.muted)
          .flatMap((x) =>
            Object.values(tx.project.notes)
              .filter((n) => n.clip === x.id && n.start >= x.offset && n.start < x.offset + x.length)
              .map((n) => ({ at: x.start + (n.start - x.offset), n, cut: x.offset + x.length - n.start })),
          )
          .filter(({ at }) => at >= f.start && at < f.end)
          .sort((a, b) => a.at - b.at || a.n.pitch - b.n.pitch);
        apply(cmd("Clip", { type: "CreateMidi", id: clip, track: id, start: f.start, length: f.end - f.start, name: t.name }));
        if (notes.length)
          apply(
            cmd("Note", {
              type: "Add",
              clip,
              notes: notes.map(({ at, n, cut }) => ({
                id: this.host.newId(),
                pitch: n.pitch,
                velocity: n.velocity,
                start: at - f.start,
                duration: Math.min(n.duration, cut, f.end - at),
              })),
            }),
          );
      } else if (t.kind === "Audio") {
        const r = media[m++];
        if (!r) continue;
        tx.upsert("Media", r);
        this.place(apply, clip, id, f.start, f.end - f.start, r.id);
      }
    }
  }

  private place(apply: (c: Command) => void, id: ClipId, track: TrackId, start: Beats, length: Beats, media: MediaId): void {
    apply(cmd("Clip", { type: "CreateAudio", id, track, start, media }));
    apply(cmd("Clip", { type: "SetBounds", id, start, length, offset: 0 }));
  }

  /** Advance the running job by one phase (called on every playhead step). */
  step(): void {
    const job = this.job;
    if (!job) return;
    job.phase += 1;
    if (job.phase === 1) {
      this.host.emit({ type: "Freeze", event: { type: "Progress", job: job.job, progress: 0.5 } });
      return;
    }
    this.job = null;
    const f = job.finish;
    try {
      this.host.transact(f.type, (tx, apply) => {
        const p = tx.project;
        if (f.type === "Freeze") {
          const t = p.tracks[f.track] ?? fail("InvalidState", "the track was deleted during the render");
          const m = job.media[0]!;
          tx.upsert("Media", m);
          tx.upsert("Track", { ...t, freeze: { media: m.id, start: 0 } });
        } else if (f.type === "Bounce") {
          const t = p.tracks[f.track] ?? fail("InvalidState", "the track was deleted during the render");
          const m = job.media[0]!;
          tx.upsert("Media", m);
          const length = (m.frames / m.sample_rate) * (bpmOf(p) / 60);
          if (f.target.type === "NewTrack") {
            const sibs = Object.values(p.tracks)
              .filter((x) => x.parent === t.parent)
              .sort((a, b) => (a.order < b.order ? -1 : a.order > b.order ? 1 : 0));
            const next = sibs[sibs.findIndex((x) => x.id === t.id) + 1];
            apply(
              cmd("Track", {
                type: "Create",
                id: f.target.track,
                kind: "Audio",
                name: `${t.name} Bounce`,
                color: t.color,
                parent: t.parent,
                before: next?.id ?? null,
              }),
            );
            this.place(apply, f.target.clip, f.target.track, f.start, length, m.id);
            if (!t.mixer.mute) apply(cmd("Mixer", { type: "SetMute", track: t.id, mute: true }));
          } else {
            this.place(apply, f.target.clip, t.id, f.start, length, m.id);
          }
        } else {
          this.consolidate(tx, apply, f, job.media);
        }
      });
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      this.host.emit({ type: "Freeze", event: { type: "Failed", job: job.job, message } });
      return;
    }
    this.host.emit({ type: "Freeze", event: { type: "Progress", job: job.job, progress: 1 } });
    this.host.emit({ type: "Freeze", event: { type: "Done", job: job.job } });
  }
}

/** Tempo at the song start (the mock places renders at a constant tempo). */
function bpmOf(p: Project): number {
  const first = Object.values(p.tempo_points).sort((a, b) => a.time - b.time)[0];
  return first?.bpm ?? 120;
}
