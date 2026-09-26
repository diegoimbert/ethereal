/**
 * Tempo-map math for the mock (step tempo only: `TempoCurve::Linear` segments are treated
 * as steps). The real tempo map lives in `ether-model`/`ether-core`.
 */

import { ceilBeats, floorBeats } from "@/state/beats";
import type { Beats, Project, Seconds, TempoPoint, TimeSignature, TimeSignaturePoint } from "@/generated";

const byTime = <T extends { time: number }>(a: T, b: T) => a.time - b.time;

export function sortedTempoPoints(project: Project): TempoPoint[] {
  return Object.values(project.tempo_points).sort(byTime);
}

export function sortedSignatures(project: Project): TimeSignaturePoint[] {
  return Object.values(project.time_signatures).sort(byTime);
}

/** The point in effect at `beats` (the last one at or before it; the first one otherwise). */
function pointAt<T extends { time: number }>(points: T[], beats: Beats): T | undefined {
  let found = points[0];
  for (const p of points) {
    if (p.time <= beats) found = p;
    else break;
  }
  return found;
}

export function tempoPointAt(project: Project, beats: Beats): TempoPoint | undefined {
  return pointAt(sortedTempoPoints(project), beats);
}

export function signaturePointAt(project: Project, beats: Beats): TimeSignaturePoint | undefined {
  return pointAt(sortedSignatures(project), beats);
}

export function bpmAt(project: Project, beats: Beats): number {
  return tempoPointAt(project, beats)?.bpm ?? 120;
}

export function signatureAt(project: Project, beats: Beats): TimeSignature {
  return signaturePointAt(project, beats)?.signature ?? { numerator: 4, denominator: 4 };
}

/** Quarter-note beats per bar of a signature (6/8 → 3). */
export function beatsPerBar(sig: TimeSignature): number {
  return (sig.numerator * 4) / sig.denominator;
}

/** Seconds from beat 0 to `beats`, integrating the (step) tempo map. */
export function beatsToSeconds(project: Project, beats: Beats): Seconds {
  const points = sortedTempoPoints(project);
  let seconds = 0;
  let pos = 0;
  let bpm = points[0]?.bpm ?? 120;
  for (const p of points) {
    if (p.time >= beats) break;
    if (p.time > pos) {
      seconds += ((p.time - pos) * 60) / bpm;
      pos = p.time;
    }
    bpm = p.bpm;
  }
  return seconds + ((beats - pos) * 60) / bpm;
}

/**
 * The next multiple of `grid` beats strictly after `position` (or at it if `inclusive`),
 * with the shared `BEATS_EPSILON` tolerance (a position within epsilon of a line is on it).
 */
export function nextGridLine(position: Beats, grid: Beats, inclusive = false): Beats {
  if (grid <= 0) return position;
  return inclusive ? ceilBeats(position, grid) : floorBeats(position, grid) + grid;
}
