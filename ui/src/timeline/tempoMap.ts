/**
 * UI-side tempo map: beats ↔ seconds and bars/beats, from the project mirror's
 * `tempo_points` and `time_signatures`.
 *
 * Mirrors Rust `ether_model::TempoMap` (`crates/ether-model/src/tempo.rs`), which is the
 * source of truth (shared vectors: `crates/ether-model/tests/tempo_vectors.json`):
 * - seconds are measured from beat 0;
 * - a `Step` segment is constant; a `Linear` segment ramps BPM linearly *over beats* to the
 *   next point's BPM;
 * - before the first point and after the last one the tempo is constant;
 * - bar/beat positions are 1-based (bar 1 beat 1 = beat 0), beats count in the
 *   signature's denominator unit (6/8 has 6 eighth-note beats); a signature change that is
 *   not on a bar line ends the previous bar early (the partial bar counts as one bar);
 *   negative positions extrapolate the first signature (bar 0, -1, ...);
 * - an empty map means 120 BPM, 4/4.
 */

import { useMemo } from "react";
import type { Beats, Project, Seconds, TempoPoint, TimeSignature, TimeSignaturePoint } from "@/generated";
import { BEATS_EPSILON, ceilBeats } from "@/state/beats";
import { useProjectStore } from "@/state/projectStore";

export const DEFAULT_BPM = 120;
export const DEFAULT_SIGNATURE: TimeSignature = { numerator: 4, denominator: 4 };

/** Quarter-note beats per bar of a signature (4/4 → 4, 6/8 → 3, 7/8 → 3.5). */
export function beatsPerBar(sig: TimeSignature): Beats {
  return (sig.numerator * 4) / sig.denominator;
}

/** Length in quarter-note beats of one beat (the signature's denominator) of `sig`. */
export function beatUnit(sig: TimeSignature): Beats {
  return 4 / sig.denominator;
}

/** Bar/beat position for display (1-based, like `ether_model::BarBeat`). */
export interface BarBeat {
  bar: number;
  /** 1-based beat within the bar, in the signature's beat unit. */
  beat: number;
  /** Fraction of the beat, 0..1. */
  fraction: number;
}

/** One bar line (start of a bar). */
export interface BarLine {
  /** 1-based bar number. */
  bar: number;
  /** Position in beats. */
  beats: Beats;
  signature: TimeSignature;
}

interface TempoSeg {
  time: Beats;
  bpm: number;
  /** BPM change per beat within the segment (0 = constant). */
  slope: number;
  /** Seconds at `time`, measured from the first point. */
  seconds: Seconds;
}

interface SigSeg {
  time: Beats;
  signature: TimeSignature;
  /** 1-based bar number at `time`. */
  bar: number;
}

const byTime = <T extends { time: number; id: string }>(a: T, b: T) =>
  a.time - b.time || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);

/** Seconds elapsed over `dx` beats into a segment. */
function segSeconds(seg: TempoSeg, dx: Beats): Seconds {
  if (seg.slope === 0) return (dx * 60) / seg.bpm;
  return (60 / seg.slope) * Math.log((seg.bpm + seg.slope * dx) / seg.bpm);
}

/** Beats elapsed after `ds` seconds into a segment. */
function segBeats(seg: TempoSeg, ds: Seconds): Beats {
  if (seg.slope === 0) return (ds * seg.bpm) / 60;
  return (seg.bpm * (Math.exp((seg.slope * ds) / 60) - 1)) / seg.slope;
}

/** Index of the last element with `key(el) <= x` (0 if none). */
function lastAtOrBefore<T>(arr: ReadonlyArray<T>, x: number, key: (el: T) => number): number {
  let lo = 0;
  let hi = arr.length - 1;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (key(arr[mid]!) <= x) lo = mid;
    else hi = mid - 1;
  }
  return lo;
}

export class TempoMap {
  private readonly tempo: TempoSeg[];
  /** Seconds from the first point to beat 0. */
  private readonly offset: Seconds;
  private readonly sigs: SigSeg[];

  constructor(tempoPoints: ReadonlyArray<TempoPoint>, signatures: ReadonlyArray<TimeSignaturePoint>) {
    const tp = [...tempoPoints].sort(byTime);
    if (tp.length === 0) tp.push({ id: "", time: 0, bpm: DEFAULT_BPM, curve: "Step" });
    this.tempo = tp.map((p, i) => {
      const next = tp[i + 1];
      const len = next ? next.time - p.time : 0;
      const ramps = p.curve === "Linear" && next !== undefined && len > BEATS_EPSILON && Math.abs(next.bpm - p.bpm) > 1e-9;
      return { time: p.time, bpm: p.bpm, slope: ramps ? (next.bpm - p.bpm) / len : 0, seconds: 0 };
    });
    for (let i = 1; i < this.tempo.length; i++) {
      const prev = this.tempo[i - 1]!;
      this.tempo[i]!.seconds = prev.seconds + segSeconds(prev, this.tempo[i]!.time - prev.time);
    }
    this.offset = this.secondsFromFirst(0);

    // Signatures: the first one applies from beat 0 (as in the model).
    const sp = [...signatures].sort(byTime);
    this.sigs = [{ time: 0, signature: sp[0]?.signature ?? DEFAULT_SIGNATURE, bar: 1 }];
    for (const p of sp.slice(1)) {
      const prev = this.sigs[this.sigs.length - 1]!;
      if (p.time <= prev.time + BEATS_EPSILON) {
        prev.signature = p.signature;
        continue;
      }
      const bpb = beatsPerBar(prev.signature);
      const bars = Math.round(ceilBeats(p.time - prev.time, bpb) / bpb);
      this.sigs.push({ time: p.time, signature: p.signature, bar: prev.bar + bars });
    }
  }

  /** Build from the project mirror. */
  static fromProject(project: Pick<Project, "tempo_points" | "time_signatures">): TempoMap {
    return new TempoMap(Object.values(project.tempo_points), Object.values(project.time_signatures));
  }

  /** Constant tempo/signature map (tests, empty states). */
  static constant(bpm = DEFAULT_BPM, signature: TimeSignature = DEFAULT_SIGNATURE): TempoMap {
    return new TempoMap([{ id: "t", time: 0, bpm, curve: "Step" }], [{ id: "s", time: 0, signature }]);
  }

  private secondsFromFirst(beats: Beats): Seconds {
    const first = this.tempo[0]!;
    if (beats <= first.time) return ((beats - first.time) * 60) / first.bpm;
    const seg = this.tempo[lastAtOrBefore(this.tempo, beats, (s) => s.time)]!;
    return seg.seconds + segSeconds(seg, beats - seg.time);
  }

  /** Instantaneous tempo at `beats` (interpolated inside linear segments). */
  bpmAt(beats: Beats): number {
    const seg = this.tempo[lastAtOrBefore(this.tempo, beats + BEATS_EPSILON, (s) => s.time)]!;
    if (seg.slope === 0) return seg.bpm;
    const i = this.tempo.indexOf(seg);
    const len = this.tempo[i + 1]!.time - seg.time;
    return seg.bpm + seg.slope * Math.min(len, Math.max(0, beats - seg.time));
  }

  /** Seconds from beat 0 (negative before it). */
  beatsToSeconds(beats: Beats): Seconds {
    return this.secondsFromFirst(beats) - this.offset;
  }

  /** Inverse of `beatsToSeconds`. */
  secondsToBeats(seconds: Seconds): Beats {
    const first = this.tempo[0]!;
    const abs = seconds + this.offset;
    if (abs <= 0) return first.time + (abs * first.bpm) / 60;
    const seg = this.tempo[lastAtOrBefore(this.tempo, abs, (s) => s.seconds)]!;
    return seg.time + segBeats(seg, abs - seg.seconds);
  }

  private sigSegAt(beats: Beats): SigSeg {
    return this.sigs[lastAtOrBefore(this.sigs, beats + BEATS_EPSILON, (s) => s.time)]!;
  }

  /** Signature in effect at `beats`. */
  signatureAt(beats: Beats): TimeSignature {
    return this.sigSegAt(beats).signature;
  }

  /** The bar containing `beats` (a position within epsilon below a bar line is on it). */
  barAt(beats: Beats): BarLine {
    const seg = this.sigSegAt(beats);
    const bpb = beatsPerBar(seg.signature);
    const n = Math.floor((beats - seg.time + BEATS_EPSILON) / bpb);
    return { bar: seg.bar + n, beats: seg.time + n * bpb, signature: seg.signature };
  }

  /** The bar after `line` (cut short by a signature change that isn't on a bar line). */
  nextBar(line: BarLine): BarLine {
    let start = line.beats + beatsPerBar(line.signature);
    const nextSig = this.sigs.find((s) => s.time > line.beats + BEATS_EPSILON);
    if (nextSig && nextSig.time < start - BEATS_EPSILON) start = nextSig.time;
    return { bar: line.bar + 1, beats: start, signature: this.sigSegAt(start).signature };
  }

  /** Start (in beats) of 1-based bar number `bar`. */
  barToBeats(bar: number): Beats {
    const seg = this.sigs[lastAtOrBefore(this.sigs, bar, (s) => s.bar)]!;
    return seg.time + (bar - seg.bar) * beatsPerBar(seg.signature);
  }

  /** 1-based bar/beat/fraction of a position. */
  barBeat(beats: Beats): BarBeat {
    const line = this.barAt(beats);
    const sig = line.signature;
    const unit = beatUnit(sig);
    const rem = Math.max(0, beats - line.beats);
    const idx = Math.min(Math.floor((rem + BEATS_EPSILON) / unit), Math.max(1, sig.numerator) - 1);
    const fraction = Math.min(1, Math.max(0, (rem - idx * unit) / unit));
    return { bar: line.bar, beat: idx + 1, fraction: fraction < 1e-6 || fraction >= 1 ? 0 : fraction };
  }

  /** Bar lines with `from <= beats < to`, every `everyBars` bars (counted from bar 1). */
  barLines(from: Beats, to: Beats, everyBars = 1): BarLine[] {
    const out: BarLine[] = [];
    const step = Math.max(1, Math.round(everyBars));
    let line = this.barAt(from);
    if (line.beats < from - BEATS_EPSILON) line = this.nextBar(line);
    // Align to bars 1, 1 + step, ...
    while ((((line.bar - 1) % step) + step) % step !== 0) line = this.nextBar(line);
    let guard = 0;
    while (line.beats < to - BEATS_EPSILON && guard++ < 100_000) {
      out.push(line);
      for (let i = 0; i < step; i++) line = this.nextBar(line);
    }
    return out;
  }
}

/** Tempo map of the current project (memoized on its tempo/signature tables). */
export function useTempoMap(): TempoMap {
  const tempo = useProjectStore((s) => s.project?.tempo_points);
  const sigs = useProjectStore((s) => s.project?.time_signatures);
  return useMemo(
    () => (tempo && sigs ? TempoMap.fromProject({ tempo_points: tempo, time_signatures: sigs }) : TempoMap.constant()),
    [tempo, sigs],
  );
}
