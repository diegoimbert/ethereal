/** Time display formatting (bars.beats.sixteenths, minutes:seconds). */

import type { Beats, Seconds } from "@/generated";
import { beatUnit, type TempoMap } from "./tempoMap";

/**
 * Ableton-style position "bar.beat.sixteenth" (all 1-based), e.g. beat 0 → "1.1.1",
 * beat 5.25 in 4/4 → "2.2.2". `parts` limits the output to "bar" or "bar.beat".
 */
export function formatBarBeat(beats: Beats, tempo: TempoMap, parts: 1 | 2 | 3 = 3): string {
  const bb = tempo.barBeat(beats);
  if (parts === 1) return `${bb.bar}`;
  if (parts === 2) return `${bb.bar}.${bb.beat}`;
  const unit = beatUnit(tempo.signatureAt(beats));
  // Sixteenths within the beat (a 1/16 note = 0.25 quarter beats).
  const sixteenth = Math.floor(bb.fraction * (unit / 0.25) + 1e-6) + 1;
  return `${bb.bar}.${bb.beat}.${sixteenth}`;
}

/** A beat length as "bars.beats.sixteenths" (0-based durations, e.g. 4 beats in 4/4 → "1.0.0"). */
export function formatDuration(length: Beats, beatsPerBarLen = 4): string {
  const bars = Math.floor((length + 1e-6) / beatsPerBarLen);
  const rest = length - bars * beatsPerBarLen;
  const beats = Math.floor(rest + 1e-6);
  const six = Math.floor((rest - beats) * 4 + 1e-6);
  return `${bars}.${beats}.${six}`;
}

/** "m:ss.mmm" (negative values get a leading "-"). `ms: false` drops the milliseconds. */
export function formatSeconds(seconds: Seconds, ms = true): string {
  const sign = seconds < 0 ? "-" : "";
  const totalMs = Math.round(Math.abs(seconds) * 1000);
  const m = Math.floor(totalMs / 60000);
  const s = Math.floor((totalMs % 60000) / 1000);
  const rest = totalMs % 1000;
  const base = `${sign}${m}:${String(s).padStart(2, "0")}`;
  return ms ? `${base}.${String(rest).padStart(3, "0")}` : base;
}
