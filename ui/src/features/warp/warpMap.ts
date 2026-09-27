/**
 * Content beat → source seconds of an audio clip, exactly as the engine plays it (pure).
 *
 * Mirrors the Rust side, which is the source of truth:
 * - `compileWarp` ↔ `ether-controller` `warp::warp_desc` (markers sorted by beat, deduped
 *   within `BEATS_EPSILON`, `source_bpm` supplying the slope when fewer than two markers;
 *   `null` = unwarped);
 * - `sourceSecondsAt` ↔ `ether-core` `sched::source_seconds` (piecewise linear, extended
 *   past the edge markers with the edge slopes; unwarped: content beats at the song tempo
 *   at the clip start);
 * - `clipSourceMapper` ↔ `ether-core` `warp::repitch_source_seconds`: Repitch (and unwarped
 *   clips, and Complex where the engine has no stretcher: the web build) plays transposed
 *   clips faster/slower around the clip offset; Complex keeps the mapping and shifts pitch.
 */

import type { AudioContent, Beats, WarpMarker, WarpSettings } from "@/generated";
import type { EngineTransport } from "@/transport";
import { beatsApproxEq } from "@/state";

/** `[content beat, source seconds]`. */
export type Pin = readonly [Beats, number];

type MarkerLike = Pick<WarpMarker, "beat" | "source">;

/** The engine's warp pins for a clip, or `null` when it plays unwarped. */
export function compileWarp(warp: WarpSettings, markers: ReadonlyArray<MarkerLike>): Pin[] | null {
  if (!warp.enabled) return null;
  const sorted = markers
    .filter((m) => Number.isFinite(m.beat) && Number.isFinite(m.source))
    .slice()
    .sort((a, b) => a.beat - b.beat);
  const pins: Pin[] = [];
  for (const m of sorted) {
    const last = pins[pins.length - 1];
    if (last && beatsApproxEq(last[0], m.beat)) continue;
    pins.push([m.beat, m.source]);
  }
  if (pins.length >= 2) return pins;
  const bpm = warp.source_bpm;
  if (bpm === null || !Number.isFinite(bpm) || bpm <= 0) return null;
  const [b0, s0] = pins[0] ?? [0, 0];
  return [
    [b0, s0],
    [b0 + 1, s0 + 60 / bpm],
  ];
}

/** Source seconds of content beat `c` (`refBpm`: song tempo at the clip start). */
export function sourceSecondsAt(pins: ReadonlyArray<Pin> | null, refBpm: number, c: Beats): number {
  if (!pins || pins.length < 2) return (c * 60) / (refBpm > 0 ? refBpm : 120);
  // First pin with beat > c, clamped to [1, n-1] (same as `partition_point(b <= c)`).
  let i = 0;
  while (i < pins.length && pins[i]![0] <= c) i++;
  i = Math.min(Math.max(i, 1), pins.length - 1);
  const [b0, s0] = pins[i - 1]!;
  const [b1, s1] = pins[i]!;
  if (b1 - b0 <= 0) return s0;
  return s0 + ((c - b0) * (s1 - s0)) / (b1 - b0);
}

/** Whether the engine behind `kind` time-stretches Complex clips (the web build doesn't). */
export function complexStretches(kind: EngineTransport["kind"]): boolean {
  return kind !== "wasm";
}

/** Playback-rate factor of a transpose in semitones. */
export function semitoneRatio(semitones: number): number {
  return Math.pow(2, semitones / 12);
}

/** Does this clip play by resampling (so transpose changes speed)? */
export function playsRepitch(content: AudioContent, pins: ReadonlyArray<Pin> | null, kind: EngineTransport["kind"]): boolean {
  return !pins || content.warp.mode === "Repitch" || !complexStretches(kind);
}

/**
 * Content beat → source seconds of an audio clip as the engine plays it.
 * `refBpm` = song tempo at the clip start, `anchor` = clip offset (content beat).
 */
export function clipSourceMapper(
  content: AudioContent,
  markers: ReadonlyArray<MarkerLike>,
  refBpm: number,
  anchor: Beats,
  kind: EngineTransport["kind"],
): (c: Beats) => number {
  const pins = compileWarp(content.warp, markers);
  const t = content.transpose;
  if (!t || !Number.isFinite(t) || !playsRepitch(content, pins, kind)) {
    return (c) => sourceSecondsAt(pins, refBpm, c);
  }
  const ratio = semitoneRatio(t);
  const s0 = sourceSecondsAt(pins, refBpm, anchor);
  return (c) => s0 + (sourceSecondsAt(pins, refBpm, c) - s0) * ratio;
}

/**
 * Content beat at which `toSeconds` reaches source second `s` (bisection; assumes the
 * mapping increases, which it does unless markers cross).
 */
export function beatAtSource(toSeconds: (c: Beats) => number, s: number): Beats {
  let lo = -1;
  let hi = 1;
  for (let i = 0; i < 64 && toSeconds(lo) > s; i++) lo *= 2;
  for (let i = 0; i < 64 && toSeconds(hi) < s; i++) hi *= 2;
  for (let i = 0; i < 60; i++) {
    const mid = (lo + hi) / 2;
    if (toSeconds(mid) < s) lo = mid;
    else hi = mid;
  }
  return (lo + hi) / 2;
}

/** Tempo (BPM) of the source between two pins at `songBpm`-independent terms: beats per minute of source. */
export function segmentBpm(a: Pin, b: Pin): number | null {
  const ds = b[1] - a[1];
  return ds > 0 ? ((b[0] - a[0]) * 60) / ds : null;
}
