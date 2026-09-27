/**
 * Live recording view (`live-record`): what is being recorded, accumulated from
 * `RecordingEvent::Progress` (only new data per event, ~20 Hz) until the real clips arrive.
 *
 * - Audio: one `LiveTake` per (track, take). Peaks are merged-channel min/max at a fixed
 *   `frames_per_peak`, placed by `first_peak` (a gap, from chunks the host dropped, reads
 *   as silence). The take starts at the latency-compensated `start` the committed clip gets.
 * - MIDI: notes per track; sent when they start (`length: null`, held: they grow to the
 *   playhead) and again when they end.
 * - `Started` clears everything. `Stopped` freezes the view ("handoff") until the committed
 *   clips' peaks are ready (`Media::PeaksReady`, plus a short grace for the tile fetch) or a
 *   timeout, so the live waveform is replaced by the real one without a blank frame.
 *
 * Runtime only (never in the document or the undo history).
 */

import type { ClipId, MediaId, PeakData, RecordingEvent, TrackId } from "@/generated";
import { TILE_PEAKS } from "@/features/arrangement/peaks";

/** After the last committed clip's `PeaksReady`: time for its tiles to be fetched (ms). */
export const HANDOFF_GRACE_MS = 300;
/** Longest the frozen live view stays after `Stopped` (ms). */
export const HANDOFF_TIMEOUT_MS = 3000;
/** Shortest live note (beats), like the committed notes. */
export const MIN_NOTE = 1 / 64;

export type LivePhase = "idle" | "recording" | "handoff";

/** Each coarser live peak level merges this many peaks of the finer one. */
export const MIP_FACTOR = 16;
/** Peak levels kept per take: 256, 4096, 65536 frames per peak (at 256 base). */
const MIP_LEVELS = 3;

/** Min/max peaks at one resolution, stored in growable typed arrays. */
class PeakLevel {
  count = 0;
  min = new Float32Array(256);
  max = new Float32Array(256);
  private tiles = new Map<number, { n: number; data: PeakData }>();

  constructor(readonly framesPerPeak: number) {}

  /** Make room for `end` peaks. */
  reserve(end: number): void {
    if (end <= this.min.length) return;
    let cap = this.min.length;
    while (cap < end) cap *= 2;
    const grow = (a: Float32Array) => {
      const b = new Float32Array(cap);
      b.set(a.subarray(0, this.count));
      return b;
    };
    this.min = grow(this.min);
    this.max = grow(this.max);
  }

  /** Peaks from index `from` on changed (tiles touching them are rebuilt on demand). */
  touched(from: number, end: number): void {
    const t0 = Math.floor(Math.min(from, this.count) / TILE_PEAKS);
    this.count = Math.max(this.count, end);
    for (const k of [...this.tiles.keys()]) if (k >= t0) this.tiles.delete(k);
  }

  tile(index: number): PeakData | null {
    const i0 = index * TILE_PEAKS;
    if (index < 0 || i0 >= this.count) return null;
    const n = Math.min(TILE_PEAKS, this.count - i0);
    const hit = this.tiles.get(index);
    if (hit && hit.n === n) return hit.data;
    // Typed-array views: `peakRange` only indexes them.
    const data: PeakData = {
      media: "",
      samples_per_peak: this.framesPerPeak,
      start_frame: i0 * this.framesPerPeak,
      min: [this.min.subarray(i0, i0 + n) as unknown as number[]],
      max: [this.max.subarray(i0, i0 + n) as unknown as number[]],
    };
    this.tiles.set(index, { n, data });
    return data;
  }
}

export class LiveTake {
  /**
   * `levels[0]` holds the received peaks; each coarser level (`MIP_FACTOR` times more
   * frames per peak) is kept up to date incrementally, so drawing a long take zoomed out
   * reads about one peak per pixel, like committed clips (`peakLevel`).
   */
  readonly levels: PeakLevel[];

  constructor(
    readonly track: TrackId,
    readonly take: number,
    readonly start: number,
    readonly sampleRate: number,
    readonly framesPerPeak: number,
  ) {
    this.levels = Array.from({ length: MIP_LEVELS }, (_, i) => new PeakLevel(framesPerPeak * MIP_FACTOR ** i));
  }

  /** Received peaks (base level). */
  get count(): number {
    return this.levels[0]!.count;
  }
  get min(): Float32Array {
    return this.levels[0]!.min;
  }
  get max(): Float32Array {
    return this.levels[0]!.max;
  }

  /** Store peaks `min`/`max` at index `first` (gaps read as silence). */
  put(first: number, min: ReadonlyArray<number>, max: ReadonlyArray<number>): void {
    const base = this.levels[0]!;
    const end = first + min.length;
    base.reserve(end);
    base.min.set(min, first);
    base.max.set(max, first);
    base.touched(first, end);
    // Coarser levels: recompute the peaks covering the change (O(new peaks)).
    let lo = Math.min(first, base.count);
    for (let l = 1; l < this.levels.length; l++) {
      const fine = this.levels[l - 1]!;
      const coarse = this.levels[l]!;
      const j0 = Math.floor(lo / MIP_FACTOR);
      const j1 = Math.ceil(fine.count / MIP_FACTOR);
      coarse.reserve(j1);
      for (let j = j0; j < j1; j++) {
        let mn = Infinity;
        let mx = -Infinity;
        const k1 = Math.min(fine.count, (j + 1) * MIP_FACTOR);
        for (let k = j * MIP_FACTOR; k < k1; k++) {
          if (fine.min[k]! < mn) mn = fine.min[k]!;
          if (fine.max[k]! > mx) mx = fine.max[k]!;
        }
        coarse.min[j] = mn;
        coarse.max[j] = mx;
      }
      coarse.touched(j0, j1);
      lo = j0;
    }
  }

  /** Recorded frames covered by the peaks so far. */
  get frames(): number {
    return this.count * this.framesPerPeak;
  }

  /** The coarsest level with at most `framesPerPx` frames per peak (0: the received peaks). */
  levelFor(framesPerPx: number): number {
    let l = 0;
    while (l + 1 < this.levels.length && this.levels[l + 1]!.framesPerPeak <= framesPerPx) l++;
    return l;
  }

  /**
   * Level `level`'s peaks as `PeakData` tiles for `drawWaveform` / `peakRange` (its
   * `framesPerPeak` is the draw level, one merged channel).
   */
  tile(index: number, level = 0): PeakData | null {
    return this.levels[level]?.tile(index) ?? null;
  }
}

export interface LiveNote {
  pitch: number;
  velocity: number;
  start: number;
  /** `null` while held. */
  length: number | null;
}

export interface LiveTrackView {
  takes: ReadonlyArray<LiveTake>;
  notes: ReadonlyArray<LiveNote>;
  phase: LivePhase;
}

const EMPTY: LiveTrackView = { takes: [], notes: [], phase: "idle" };

type Listener = () => void;

export class LiveRecording {
  phase: LivePhase = "idle";
  private takes = new Map<string, LiveTake>();
  private notes = new Map<TrackId, LiveNote[]>();
  /** Per-track snapshots (new object on change, for `useSyncExternalStore`). */
  private views = new Map<TrackId, LiveTrackView>();
  private listeners = new Set<Listener>();
  /** Media whose peaks became ready since `Started`. */
  private ready = new Set<MediaId>();
  /** Committed media still waiting for `PeaksReady` during the handoff. */
  private waiting = new Set<MediaId>();
  private timer: ReturnType<typeof setTimeout> | null = null;

  constructor(
    /** Media of a committed clip (audio) or `null`. */
    private readonly mediaOf: (clip: ClipId) => MediaId | null = () => null,
  ) {}

  subscribe(l: Listener): () => void {
    this.listeners.add(l);
    return () => this.listeners.delete(l);
  }

  view(track: TrackId): LiveTrackView {
    return this.views.get(track) ?? EMPTY;
  }

  /** Apply a recording event; returns whether the live view changed. */
  apply(ev: RecordingEvent): boolean {
    switch (ev.type) {
      case "Started":
        this.reset();
        this.phase = "recording";
        this.refresh(null);
        return true;
      case "Progress":
        if (this.phase !== "recording") return false;
        this.progress(ev.audio, ev.midi);
        return true;
      case "Stopped":
        return this.stopped(ev.clips);
      default:
        return false;
    }
  }

  /** `Media::PeaksReady`. */
  peaksReady(media: MediaId): void {
    this.ready.add(media);
    if (this.phase === "handoff" && this.waiting.delete(media) && this.waiting.size === 0) this.endHandoffSoon(HANDOFF_GRACE_MS);
  }

  /** Drop everything (and cancel a pending handoff). */
  reset(): void {
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = null;
    const had = this.phase !== "idle" || this.takes.size > 0 || this.notes.size > 0;
    this.takes.clear();
    this.notes.clear();
    this.ready.clear();
    this.waiting.clear();
    this.phase = "idle";
    if (had) this.refresh(null);
  }

  private progress(audio: Extract<RecordingEvent, { type: "Progress" }>["audio"], midi: Extract<RecordingEvent, { type: "Progress" }>["midi"]) {
    const changed = new Set<TrackId>();
    for (const c of audio) {
      const key = `${c.track}/${c.take}`;
      let t = this.takes.get(key);
      if (!t) {
        t = new LiveTake(c.track, c.take, c.start, c.sample_rate, c.frames_per_peak);
        this.takes.set(key, t);
      }
      t.put(c.first_peak, c.min, c.max);
      changed.add(c.track);
    }
    for (const n of midi) {
      let list = this.notes.get(n.track);
      if (!list) this.notes.set(n.track, (list = []));
      const held = n.length === null ? undefined : list.find((h) => h.length === null && h.pitch === n.pitch && Math.abs(h.start - n.start) < 1e-9);
      if (held) held.length = n.length;
      else list.push({ pitch: n.pitch, velocity: n.velocity, start: n.start, length: n.length });
      changed.add(n.track);
    }
    this.refresh(changed);
  }

  private stopped(clips: ReadonlyArray<ClipId>): boolean {
    if (this.phase !== "recording") return false;
    const media = clips.map(this.mediaOf).filter((m): m is MediaId => m !== null && !this.ready.has(m));
    this.phase = "handoff";
    this.waiting = new Set(media);
    // Nothing to wait for: MIDI clips and ready peaks appear with the patch.
    this.endHandoffSoon(media.length === 0 ? HANDOFF_GRACE_MS : HANDOFF_TIMEOUT_MS);
    this.refresh(null);
    return true;
  }

  private endHandoffSoon(ms: number) {
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = setTimeout(() => {
      this.timer = null;
      if (this.phase === "handoff") this.reset();
    }, ms);
  }

  /** New snapshots for `tracks` (all when `null`), then notify. */
  private refresh(tracks: Set<TrackId> | null) {
    const all = new Set<TrackId>([...this.views.keys()]);
    for (const t of this.takes.values()) all.add(t.track);
    for (const k of this.notes.keys()) all.add(k);
    for (const track of all) {
      if (tracks && !tracks.has(track)) continue;
      const takes = [...this.takes.values()].filter((t) => t.track === track).sort((a, b) => a.take - b.take);
      const notes = this.notes.get(track) ?? [];
      if (takes.length === 0 && notes.length === 0) this.views.delete(track);
      else this.views.set(track, { takes, notes: [...notes], phase: this.phase });
    }
    for (const l of this.listeners) l();
  }
}

/** Timeline end (beats) of a take's peaks, at the clip's reference tempo (`bpm`). */
export function takeEnd(take: LiveTake, bpm: number): number {
  return take.start + ((take.frames / take.sampleRate) * bpm) / 60;
}

/**
 * Where a live clip ends: at the playhead while it grows (recording, playhead at or past
 * the data within `slack` beats, i.e. transport latency), else where its data ends (an
 * earlier loop take, after a punch-out, or frozen during the handoff).
 */
export function liveEnd(dataEnd: number, playhead: number | null, growing: boolean, slack: number): number {
  if (growing && playhead !== null && playhead >= dataEnd && playhead - dataEnd <= slack) return playhead;
  return dataEnd;
}

/** Notes as drawn: held notes end at `now` (at least `MIN_NOTE` long). */
export function noteSpans(notes: ReadonlyArray<LiveNote>, now: number): Array<{ pitch: number; velocity: number; start: number; end: number }> {
  return notes.map((n) => ({
    pitch: n.pitch,
    velocity: n.velocity / 127,
    start: n.start,
    end: n.start + (n.length ?? Math.max(MIN_NOTE, now - n.start)),
  }));
}
