// Stream clock anchors of the web host (docs/COLLAB.md §9.4 "Host math (web)"). Pure.
//
// Two relations, both in the engine's **context frames**:
// - the tap clock (./tapClock.ts): the tap output at context frame `tapFrame` carries the
//   timeline at `position` (render frame + graph latency: the latency rule of §9.4);
// - the RTP clock of one peer connection: `rtp = offset + frame * 48000 / sampleRate`
//   (mod 2^32). The browser picks the RTP timestamps; we observe them on the encoded frames
//   (read-only sender transform) at `performance.now()`, map that instant to the context
//   frame being rendered (`AudioContext.getOutputTimestamp()` + output latency) and subtract
//   the encoder pipeline delay (one 20 ms Opus frame + 10 ms). `offset` is the median of
//   recent observations (RTP and context frames tick on the same audio clock: no drift).
//
// An anchor is then `{ rtp: offset + tapFrame * 48000 / sampleRate, position, ... }`.

import type { StreamClock } from "@/generated";
import type { TapState } from "./tapClock";

/** RTP clock rate of the stream (Opus). */
export const RTP_RATE = 48_000;
/** Encoder pipeline delay: one 20 ms Opus frame + 10 ms (docs/COLLAB.md §9.4). */
export const ENCODER_DELAY_S = 0.03;
/** Anchors per listener while nothing jumps (`STREAM_CLOCK_INTERVAL_MS`). */
export const STREAM_CLOCK_INTERVAL_MS = 100;

const U32 = 2 ** 32;

/** `x` mod 2^32, for integral `x` (possibly negative or above 2^32). */
export function u32(x: number): number {
  const r = x % U32;
  return r < 0 ? r + U32 : r;
}

/** `(a - b)` as a wrapping i32 difference (RTP timestamps wrap every 24.8 h at 48 kHz). */
export function rtpDiff(a: number, b: number): number {
  const d = u32(a - b);
  return d >= U32 / 2 ? d - U32 : d;
}

/** `AudioContext.getOutputTimestamp()`: `contextTime` (s) was heard at `performanceTime` (ms). */
export interface OutputTimestamp {
  contextTime: number;
  performanceTime: number;
}

/**
 * Context time (s) being **rendered** at performance time `perfMs`: the sample heard then
 * plus the output latency (the render runs ahead of the device by it).
 */
export function renderContextTime(perfMs: number, ts: OutputTimestamp, outputLatencyS: number): number {
  return ts.contextTime + (perfMs - ts.performanceTime) / 1000 + outputLatencyS;
}

/** Context frame of the first sample of an encoded frame observed at `perfMs`. */
export function encodedFrameStart(perfMs: number, ts: OutputTimestamp, outputLatencyS: number, sampleRate: number): number {
  return (renderContextTime(perfMs, ts, outputLatencyS) - ENCODER_DELAY_S) * sampleRate;
}

/** RTP ↔ context frame offset of one peer connection (median of recent observations). */
export class RtpOffsetEstimator {
  private ref: number | null = null;
  private readonly diffs: number[] = [];

  constructor(
    private readonly sampleRate: number,
    private readonly window = 50,
  ) {}

  /** One encoded frame: RTP `rtp` starts at context frame `frame`. */
  observe(rtp: number, frame: number): void {
    const off = u32(Math.round(rtp - (frame * RTP_RATE) / this.sampleRate));
    if (this.ref === null) this.ref = off;
    this.diffs.push(rtpDiff(off, this.ref));
    if (this.diffs.length > this.window) this.diffs.shift();
  }

  /** The current estimate, or `null` before the first observation. */
  offset(): number | null {
    if (this.ref === null) return null;
    const sorted = [...this.diffs].sort((a, b) => a - b);
    return u32(this.ref + sorted[sorted.length >> 1]!);
  }

  /** RTP timestamp of context frame `frame`. */
  rtpAt(frame: number): number | null {
    const off = this.offset();
    return off === null ? null : u32(off + Math.round((frame * RTP_RATE) / this.sampleRate));
  }
}

/** Host transport state the listeners mirror (from the project store). */
export interface HostTransport {
  loop_enabled: boolean;
  loop_region: { start: number; end: number };
  metronome: boolean;
}

/** One anchor for `state` in the RTP stream whose RTP timestamp of context frame 0 is `offset`. */
export function anchor(
  state: TapState,
  rtp: number,
  transport: HostTransport,
  discontinuity: boolean,
  countInEnd: number | null,
): StreamClock {
  const clock: StreamClock = {
    rtp,
    position: state.position,
    playing: state.playing,
    recording: state.recording,
    bpm: state.bpm,
    loop_enabled: transport.loop_enabled,
    loop_region: transport.loop_region,
    metronome: transport.metronome,
    discontinuity,
  };
  if (countInEnd !== null) clock.count_in_end = countInEnd;
  return clock;
}

/**
 * The record start while a count-in runs (`count_in_end`, docs/COLLAB.md §9.4). Recording
 * from stopped with a count-in, the controller arms recording, locates to `start -
 * pre-roll` and plays: playback starting (from stopped) while recording is that pre-roll,
 * so `count_in_end = its first position + pre-roll`. Cleared when recording or playback
 * stops.
 */
export class CountInTracker {
  private playing = false;
  private end: number | null = null;

  /**
   * Feed tap states in order (at least every discontinuity); `preRoll` = the count-in in
   * beats (`count_in_bars` whole bars; 0 = none). Returns the current `count_in_end`.
   */
  update(state: TapState, preRoll: number): number | null {
    if (!state.recording || !state.playing) this.end = null;
    else if (!this.playing && preRoll > 0) this.end = state.position + preRoll;
    this.playing = state.playing;
    return this.end;
  }
}

/** Beats of `bars` whole bars in `numerator/denominator`. */
export function barsToBeats(bars: number, sig: { numerator: number; denominator: number }): number {
  return (bars * sig.numerator * 4) / Math.max(1, sig.denominator);
}
