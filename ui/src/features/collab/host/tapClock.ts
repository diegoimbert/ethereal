// Tap clock (web host, docs/COLLAB.md §9.4 "Host math (web)"): the engine worklet publishes
// the transport state of the stream tap it outputs (worklet output 1) in a small
// SharedArrayBuffer, in **context frames**, and the UI sender reads it to build anchors.
//
// Written by `apps/web/src/engine/engine.worklet.ts` every render quantum (no allocation),
// from the header `crates/ether-wasm/src/worklet.rs` fills (`tap_clock` layout, mirrored in
// `HEADER` below). Read on the main thread with a seqlock. No imports: the worklet bundles
// this file.

/** `crates/ether-wasm/src/worklet.rs::tap_clock` (f64 slots of the per-render header). */
export const HEADER = {
  BLOCKS: 0,
  OFFSET: 1,
  POSITION: 2,
  PLAYING: 3,
  RECORDING: 4,
  BPM: 5,
  LATENCY: 6,
  JUMPED: 7,
  JUMP_OFFSET: 8,
  LEN: 14,
} as const;
/** `worklet.rs::TAP_CLOCK_SLOT`: `output_ptr(TAP_CLOCK_SLOT)` points at the header. */
export const TAP_CLOCK_SLOT = 4;
/** `output_ptr(TAP_OUTPUT_BASE + c)`: tap channel `c`. */
export const TAP_OUTPUT_BASE = 2;

/** Transport state of the tap at one context frame. */
export interface TapState {
  /**
   * Context frame at which the tap output carries the timeline at `position` (the block's
   * render frame plus the graph latency, docs/COLLAB.md §9.4).
   */
  tapFrame: number;
  position: number;
  playing: boolean;
  recording: boolean;
  bpm: number;
  /** Graph latency (samples). */
  latency: number;
}

export interface TapClock {
  sampleRate: number;
  /** The last tapped block. */
  latest: TapState;
  /** Discontinuities seen so far (jump, gap, play/stop, latency change). */
  events: number;
  /** The last discontinuity (meaningful when `events > 0`). */
  event: TapState;
}

// Buffer layout: Int32 slot 0 = sequence (odd while writing); f64 slots from index 1.
const F_RATE = 1;
const F_LATEST = 2;
const F_EVENTS = 8;
const F_EVENT = 9;
const F_LEN = 15;
const STATE_LEN = 6;

export const TAP_CLOCK_BYTES = F_LEN * 8;

export function tapClockBuffer(): SharedArrayBuffer {
  return new SharedArrayBuffer(TAP_CLOCK_BYTES);
}

/** Worklet side. `write` allocates nothing. */
export class TapClockWriter {
  private readonly seq: Int32Array;
  private readonly f: Float64Array;
  private playing = false;
  private latency = -1;
  private events = 0;

  constructor(buffer: SharedArrayBuffer, sampleRate: number) {
    this.seq = new Int32Array(buffer, 0, 2);
    this.f = new Float64Array(buffer);
    this.f[F_RATE] = sampleRate;
  }

  /**
   * Publish the header of one render (`frame` = context frame of the quantum's first
   * sample, `currentFrame` in the worklet).
   */
  write(frame: number, h: Float64Array): void {
    if (!(h[HEADER.BLOCKS]! > 0)) return;
    const playing = h[HEADER.PLAYING] === 1;
    const latency = h[HEADER.LATENCY]!;
    const jumped = h[HEADER.JUMPED] === 1;
    const event = jumped || playing !== this.playing || latency !== this.latency;
    this.playing = playing;
    this.latency = latency;
    Atomics.add(this.seq, 0, 1);
    this.put(F_LATEST, frame, h, HEADER.OFFSET);
    if (event) {
      this.events += 1;
      this.f[F_EVENTS] = this.events;
      this.put(F_EVENT, frame, h, jumped ? HEADER.JUMP_OFFSET : HEADER.OFFSET);
    }
    Atomics.add(this.seq, 0, 1);
  }

  private put(at: number, frame: number, h: Float64Array, from: number): void {
    const f = this.f;
    // header: offset, position, playing, recording, bpm, latency
    f[at] = frame + h[from]! + h[from + 5]!;
    for (let i = 1; i < STATE_LEN; i++) f[at + i] = h[from + i]!;
  }
}

function state(f: Float64Array, at: number): TapState {
  return {
    tapFrame: f[at]!,
    position: f[at + 1]!,
    playing: f[at + 2] === 1,
    recording: f[at + 3] === 1,
    bpm: f[at + 4]!,
    latency: f[at + 5]!,
  };
}

/** Main-thread side: a consistent snapshot, or `null` before the first tapped block. */
export function readTapClock(buffer: SharedArrayBuffer): TapClock | null {
  const seq = new Int32Array(buffer, 0, 2);
  const f = new Float64Array(buffer);
  for (let attempt = 0; attempt < 8; attempt++) {
    const before = Atomics.load(seq, 0);
    if (before % 2 !== 0) continue;
    const snap: TapClock = {
      sampleRate: f[F_RATE]!,
      latest: state(f, F_LATEST),
      events: f[F_EVENTS]!,
      event: state(f, F_EVENT),
    };
    if (Atomics.load(seq, 0) === before) return before === 0 ? null : snap;
  }
  return null;
}
