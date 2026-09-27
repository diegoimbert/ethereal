// Stream clock mapping (docs/COLLAB.md §9.4): what the listener's playhead shows is what it
// hears. The host sends anchors "RTP timestamp `rtp` (48 kHz, wrapping u32) is the timeline
// at `position`"; the receiver reports which RTP timestamp it is playing
// (`getSynchronizationSources()`); this module maps one onto the other. Pure: no DOM.
import type { StreamClock } from "@/generated";

/** RTP clock rate of the stream (Opus: always 48 kHz). */
export const RTP_RATE = 48_000;
/** Anchors kept (older ones than the one in use are dropped). */
export const MAX_ANCHORS = 32;

/** `(a - b) as i32`: the signed distance between two wrapping u32 RTP timestamps. */
export function rtpDiff(a: number, b: number): number {
  return (a - b) | 0;
}

/** `a + samples` as a wrapping u32 RTP timestamp. */
export function rtpAdd(a: number, samples: number): number {
  return (a + Math.round(samples)) >>> 0;
}

/** Beats ↔ seconds with the replicated tempo map (`TempoMap` satisfies it). */
export interface TempoLike {
  beatsToSeconds(beats: number): number;
  secondsToBeats(seconds: number): number;
}

/** A synchronization source as reported by `RTCRtpReceiver.getSynchronizationSources()`. */
export interface SyncSource {
  /** RTP timestamp of the last frame delivered for playout (absent in old browsers). */
  rtpTimestamp?: number;
  /** When it was delivered, `performance.timeOrigin + performance.now()` based (ms). */
  timestamp: number;
}

/**
 * The RTP timestamp being heard now: the last delivered frame, extrapolated to `nowMs`
 * (same clock as `src.timestamp`), minus the output latency. `null` without `rtpTimestamp`.
 */
export function playoutRtp(src: SyncSource, nowMs: number, outputLatencySec: number): number | null {
  if (typeof src.rtpTimestamp !== "number") return null;
  const since = Math.max(0, nowMs - src.timestamp) / 1000;
  return rtpAdd(src.rtpTimestamp, since * RTP_RATE - Math.max(0, outputLatencySec) * RTP_RATE);
}

/** Where the listener is in the host's timeline. */
export interface MappedPosition {
  position: number;
  playing: boolean;
  recording: boolean;
  /** Before the record start of an armed count-in (`count_in_end`). */
  countIn: boolean;
  bpm: number;
  /** The anchor in use. */
  anchor: StreamClock;
}

/**
 * Keeps the host's anchors ordered by RTP time and maps a heard RTP timestamp to a song
 * position: the latest anchor at or before it (a `discontinuity` anchor is therefore never
 * interpolated across: the position jumps exactly when its first sample plays), advanced
 * by the audio time since, with a predicted loop wrap.
 */
export class StreamClockMapper {
  private anchors: StreamClock[] = [];
  /** Local arrival time of the latest anchor (ms), for the fallback mapping. */
  private lastArrival: { anchor: StreamClock; atMs: number } | null = null;

  get size(): number {
    return this.anchors.length;
  }

  reset(): void {
    this.anchors = [];
    this.lastArrival = null;
  }

  push(anchor: StreamClock, arrivalMs = 0): void {
    const a = this.anchors;
    // Insert by RTP order (wrap-aware: distances from the new anchor).
    let i = a.length;
    while (i > 0 && rtpDiff(a[i - 1]!.rtp, anchor.rtp) > 0) i--;
    // A resent anchor for the same RTP time replaces the old one.
    if (i > 0 && a[i - 1]!.rtp === anchor.rtp) a[i - 1] = anchor;
    else a.splice(i, 0, anchor);
    if (a.length > MAX_ANCHORS) a.splice(0, a.length - MAX_ANCHORS);
    if (!this.lastArrival || rtpDiff(anchor.rtp, this.lastArrival.anchor.rtp) >= 0) this.lastArrival = { anchor, atMs: arrivalMs };
  }

  /** The position heard at RTP time `rNow` (`null` before any anchor). */
  map(rNow: number, tempo?: TempoLike | null): MappedPosition | null {
    const a = this.anchors;
    if (a.length === 0) return null;
    let k = -1;
    for (let i = a.length - 1; i >= 0; i--) {
      if (rtpDiff(rNow, a[i]!.rtp) >= 0) {
        k = i;
        break;
      }
    }
    if (k < 0) {
      // Audio from before the first anchor (they usually arrive early): hold its position.
      return describe(a[0]!, a[0]!.position);
    }
    if (k > 0) a.splice(0, k); // older anchors are never needed again
    const anchor = a[0]!;
    const dt = rtpDiff(rNow, anchor.rtp) / RTP_RATE;
    return describe(anchor, advance(anchor, dt, tempo));
  }

  /**
   * Fallback without `rtpTimestamp` (§9.4): the latest anchor extrapolated by local time
   * since it arrived, minus the estimated receive delay (jitter buffer + half the RTT).
   */
  mapByArrival(nowMs: number, delaySec: number, tempo?: TempoLike | null): MappedPosition | null {
    const last = this.lastArrival;
    if (!last) return null;
    const rNow = rtpAdd(last.anchor.rtp, ((nowMs - last.atMs) / 1000 - Math.max(0, delaySec)) * RTP_RATE);
    return this.map(rNow, tempo);
  }
}

/** `anchor.position` advanced by `dt` seconds of audio (tempo map when possible). */
function advance(anchor: StreamClock, dt: number, tempo?: TempoLike | null): number {
  if (!anchor.playing) return anchor.position;
  let p: number;
  if (tempo) {
    // Negative (count-in pre-roll) positions extend the first tempo backwards.
    p = tempo.secondsToBeats(tempo.beatsToSeconds(anchor.position) + dt);
  } else {
    p = anchor.position + (dt * anchor.bpm) / 60;
  }
  const { start, end } = anchor.loop_region;
  if (anchor.loop_enabled && end > start && anchor.position < end && p >= end) {
    // Predicted wrap; the wrap anchor (a discontinuity) makes it exact.
    p = start + mod(p - start, end - start);
  }
  return p;
}

function mod(a: number, n: number): number {
  return ((a % n) + n) % n;
}

function describe(anchor: StreamClock, position: number): MappedPosition {
  const end = anchor.count_in_end;
  return {
    position,
    playing: anchor.playing,
    recording: anchor.recording,
    countIn: end !== undefined && end !== null && position < end,
    bpm: anchor.bpm,
    anchor,
  };
}
