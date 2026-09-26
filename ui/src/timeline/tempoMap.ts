/**
 * UI-side tempo map: beats ↔ seconds and bars/beats, from the project mirror's
 * `tempo_points` and `time_signatures`.
 *
 * Mirrors the semantics of Rust `ether_model::TempoMap` (`crates/ether-model/src/tempo.rs`):
 * - tempo `Step` segments hold their BPM until the next point; `Linear` segments ramp the
 *   BPM linearly over beats to the next point's BPM (the last segment is always constant);
 * - bar/beat positions are 1-based (bar 1 beat 1 = beat 0); bars before beat 0 are <= 0;
 * - a time signature applies from its point onwards; `time` falls on a bar line of the
 *   previous signature.
 * Missing points fall back to 120 BPM and 4/4 (the document always has one of each at 0).
 */

import { useMemo } from "react";
import type { Beats, Project, Seconds, TempoPoint, TimeSignature, TimeSignaturePoint } from "@/generated";
import { BEATS_EPSILON, floorBeats } from "@/state/beats";
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
  /** BPM change per beat within the segment (0 for steps). */
  slope: number;
  /** Seconds at `time`. */
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

/** Seconds elapsed over `dx` beats from a segment start at `bpm` with `slope` BPM/beat. */
function segSeconds(bpm: number, slope: number, dx: Beats): Seconds {
  if (Math.abs(slope) < 1e-12) return (dx * 60) / bpm;
  return (60 / slope) * Math.log((bpm + slope * dx) / bpm);
}

/** Beats elapsed after `ds` seconds from a segment start at `bpm` with `slope` BPM/beat. */
function segBeats(bpm: number, slope: number, ds: Seconds): Beats {
  if (Math.abs(slope) < 1e-12) return (ds * bpm) / 60;
  return (bpm * (Math.exp((slope * ds) / 60) - 1)) / slope;
}

export class TempoMap {
  private readonly tempo: TempoSeg[];
  private readonly sigs: SigSeg[];

  constructor(tempoPoints: ReadonlyArray<TempoPoint>, signatures: ReadonlyArray<TimeSignaturePoint>) {
    const tp = [...tempoPoints].sort(byTime);
    this.tempo = [];
    if (tp.length === 0 || tp[0]!.time > 0) this.tempo.push({ time: 0, bpm: tp[0]?.bpm ?? DEFAULT_BPM, slope: 0, seconds: 0 });
    for (let i = 0; i < tp.length; i++) {
      const p = tp[i]!;
      const next = tp[i + 1];
      const slope = p.curve === "Linear" && next && next.time > p.time ? (next.bpm - p.bpm) / (next.time - p.time) : 0;
      this.tempo.push({ time: Math.max(0, p.time), bpm: p.bpm, slope, seconds: 0 });
    }
    for (let i = 1; i < this.tempo.length; i++) {
      const prev = this.tempo[i - 1]!;
      const seg = this.tempo[i]!;
      seg.seconds = prev.seconds + segSeconds(prev.bpm, prev.slope, seg.time - prev.time);
    }

    const sp = [...signatures].sort(byTime);
    this.sigs = [];
    if (sp.length === 0 || sp[0]!.time > 0) this.sigs.push({ time: 0, signature: sp[0]?.signature ?? DEFAULT_SIGNATURE, bar: 1 });
    for (const p of sp) {
      const prev = this.sigs[this.sigs.length - 1];
      let bar = 1;
      if (prev) {
        if (p.time <= prev.time) {
          // Duplicate at the same time: the later one wins.
          prev.signature = p.signature;
          continue;
        }
        bar = prev.bar + Math.round((p.time - prev.time) / beatsPerBar(prev.signature));
      }
      this.sigs.push({ time: Math.max(0, p.time), signature: p.signature, bar });
    }
  }

  /** Build from the project mirror. */
  static fromProject(project: Pick<Project, "tempo_points" | "time_signatures">): TempoMap {
    return new TempoMap(Object.values(project.tempo_points), Object.values(project.time_signatures));
  }

  /** Constant tempo/signature map (tests, empty states). */
  static constant(bpm = DEFAULT_BPM, signature: TimeSignature = DEFAULT_SIGNATURE): TempoMap {
    return new TempoMap(
      [{ id: "t", time: 0, bpm, curve: "Step" }],
      [{ id: "s", time: 0, signature }],
    );
  }

  private tempoSegAt(beats: Beats): TempoSeg {
    let lo = 0;
    let hi = this.tempo.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (this.tempo[mid]!.time <= beats) lo = mid;
      else hi = mid - 1;
    }
    return this.tempo[lo]!;
  }

  private sigSegAt(beats: Beats): SigSeg {
    let found = this.sigs[0]!;
    for (const s of this.sigs) {
      if (s.time <= beats + BEATS_EPSILON) found = s;
      else break;
    }
    return found;
  }

  bpmAt(beats: Beats): number {
    const s = this.tempoSegAt(beats);
    return s.bpm + s.slope * Math.max(0, beats - s.time);
  }

  /** Seconds from beat 0. Negative beats extrapolate the initial tempo. */
  beatsToSeconds(beats: Beats): Seconds {
    const s = this.tempoSegAt(beats);
    return s.seconds + segSeconds(s.bpm, s.slope, beats - s.time);
  }

  /** Inverse of `beatsToSeconds`. */
  secondsToBeats(seconds: Seconds): Beats {
    let lo = 0;
    let hi = this.tempo.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (this.tempo[mid]!.seconds <= seconds) lo = mid;
      else hi = mid - 1;
    }
    const s = this.tempo[lo]!;
    return s.time + segBeats(s.bpm, s.slope, seconds - s.seconds);
  }

  signatureAt(beats: Beats): TimeSignature {
    return this.sigSegAt(beats).signature;
  }

  /** The bar containing `beats` (bar lines within epsilon count as the next bar). */
  barAt(beats: Beats): BarLine {
    const seg = this.sigSegAt(beats);
    const len = beatsPerBar(seg.signature);
    const start = seg.time + floorBeats(beats - seg.time, len);
    return { bar: seg.bar + Math.round((start - seg.time) / len), beats: start, signature: seg.signature };
  }

  /** Start (in beats) of 1-based bar number `bar`. */
  barToBeats(bar: number): Beats {
    let seg = this.sigs[0]!;
    for (const s of this.sigs) {
      if (s.bar <= bar) seg = s;
      else break;
    }
    return seg.time + (bar - seg.bar) * beatsPerBar(seg.signature);
  }

  /** 1-based bar/beat/fraction of a position. */
  barBeat(beats: Beats): BarBeat {
    const bar = this.barAt(beats);
    const unit = beatUnit(bar.signature);
    const inBar = Math.max(0, beats - bar.beats);
    const beatIdx = Math.floor((inBar + BEATS_EPSILON) / unit);
    const fraction = Math.max(0, (inBar - beatIdx * unit) / unit);
    return { bar: bar.bar, beat: beatIdx + 1, fraction: fraction < BEATS_EPSILON ? 0 : fraction };
  }

  /** Bar lines with `from <= beats < to`, every `everyBars` bars (counted from bar 1). */
  barLines(from: Beats, to: Beats, everyBars = 1): BarLine[] {
    const out: BarLine[] = [];
    const step = Math.max(1, Math.round(everyBars));
    let line = this.barAt(from);
    if (line.beats < from - BEATS_EPSILON) line = this.nextBar(line);
    // Align to a multiple of `step` (bar 1, 1+step, ...).
    while ((line.bar - 1) % step !== 0) line = this.nextBar(line);
    let guard = 0;
    while (line.beats < to - BEATS_EPSILON && guard++ < 100_000) {
      out.push(line);
      for (let i = 0; i < step; i++) line = this.nextBar(line);
    }
    return out;
  }

  /** The bar after `line`. */
  nextBar(line: BarLine): BarLine {
    const start = line.beats + beatsPerBar(line.signature);
    const seg = this.sigSegAt(start);
    return { bar: line.bar + 1, beats: start, signature: seg.signature };
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
