// Pure formatting/parsing helpers for the transport bar (tested in format.test.ts).
import type { Beats, TimeSignature, TimeSignaturePoint } from "@/generated";
import { BEATS_EPSILON } from "@/state";

/** Tempo range accepted by the engine (the model clamps to it). */
export const MIN_BPM = 20;
export const MAX_BPM = 999;

const DEFAULT_SIGNATURE: TimeSignature = { numerator: 4, denominator: 4 };
const VALID_DENOMINATORS = [1, 2, 4, 8, 16, 32];

/** Quarter-note beats per bar of a signature (6/8 → 3). */
export function beatsPerBar(sig: TimeSignature): number {
  return (sig.numerator * 4) / sig.denominator;
}

export interface BarPosition {
  /** 1-based bar number. */
  bar: number;
  /** 1-based beat within the bar, in units of the signature's denominator. */
  beat: number;
  /** 1-based sixteenth within the beat. */
  sixteenth: number;
}

/**
 * Musical position (bar.beat.sixteenth, beat and sixteenth 1-based) of `beats`, following the
 * time-signature map (`points` in any order; a missing point at 0 means 4/4 until the
 * first one). Values within `BEATS_EPSILON` of a grid line count as on it. Positions before
 * zero (a count-in) count bars backwards in the signature at 0, like a DAW: -1.1.1 is one
 * bar before 1.1.1 (there is no bar 0).
 */
export function barPosition(beats: Beats, points: ReadonlyArray<TimeSignaturePoint>): BarPosition {
  const sorted = [...points].sort((a, b) => a.time - b.time);
  if (sorted.length === 0 || sorted[0]!.time > BEATS_EPSILON) {
    sorted.unshift({ id: "", time: 0, signature: DEFAULT_SIGNATURE });
  }
  if (beats < -BEATS_EPSILON) {
    const signature = sorted[0]!.signature;
    const bpb = beatsPerBar(signature);
    const barsBack = Math.max(1, Math.ceil((-beats - BEATS_EPSILON) / bpb));
    return { bar: -barsBack, ...beatInBar(beats + barsBack * bpb, signature) };
  }
  const pos = Math.max(0, beats);
  let barsBefore = 0;
  for (let i = 0; i < sorted.length; i++) {
    const { time: start, signature } = sorted[i]!;
    const end = sorted[i + 1]?.time ?? Infinity;
    const bpb = beatsPerBar(signature);
    if (pos + BEATS_EPSILON < end) {
      const within = pos - start;
      const bars = Math.floor((within + BEATS_EPSILON) / bpb);
      return { bar: barsBefore + bars + 1, ...beatInBar(within - bars * bpb, signature) };
    }
    // Signature changes fall on bar lines; a partial last bar still counts as one.
    barsBefore += Math.ceil((end - start - BEATS_EPSILON) / bpb);
  }
  return { bar: barsBefore + 1, beat: 1, sixteenth: 1 };
}

/** 1-based beat and sixteenth of a position `inBar` beats into a bar of `signature`. */
function beatInBar(inBar: Beats, signature: TimeSignature): Omit<BarPosition, "bar"> {
  const offset = Math.max(0, inBar);
  const unit = 4 / signature.denominator;
  const beat = Math.min(signature.numerator - 1, Math.floor((offset + BEATS_EPSILON) / unit));
  const inBeat = Math.max(0, offset - beat * unit);
  const sixteenths = Math.max(1, Math.round(unit / 0.25));
  const sixteenth = Math.min(sixteenths - 1, Math.floor((inBeat + BEATS_EPSILON) / 0.25));
  return { beat: beat + 1, sixteenth: sixteenth + 1 };
}

/** `"12.3.1"` */
export function formatBarPosition(p: BarPosition): string {
  return `${p.bar}.${p.beat}.${p.sixteenth}`;
}

/** `"1:05.250"` (minutes:seconds.millis); before zero (a count-in) `"-0:01.500"`. */
export function formatSeconds(seconds: number): string {
  const sign = seconds < 0 ? "-" : "";
  const totalMs = Math.floor(Math.abs(seconds) * 1000 + 1e-6);
  if (totalMs === 0) return "0:00.000";
  const m = Math.floor(totalMs / 60000);
  const s = Math.floor((totalMs % 60000) / 1000);
  const ms = totalMs % 1000;
  return `${sign}${m}:${String(s).padStart(2, "0")}.${String(ms).padStart(3, "0")}`;
}

/** `"120.00"` */
export function formatBpm(bpm: number): string {
  return bpm.toFixed(2);
}

/** Parse a tempo entry; `null` if not a number in `MIN_BPM..=MAX_BPM`. Rounded to 0.01. */
export function parseBpm(text: string): number | null {
  const t = text.trim().replace(",", ".");
  if (!/^\d+(\.\d*)?$|^\.\d+$/.test(t)) return null;
  const bpm = Math.round(Number(t) * 100) / 100;
  return bpm >= MIN_BPM && bpm <= MAX_BPM ? bpm : null;
}

/** Clamp a tempo into the accepted range, rounded to 0.01. */
export function clampBpm(bpm: number): number {
  return Math.round(Math.min(MAX_BPM, Math.max(MIN_BPM, bpm)) * 100) / 100;
}

/** `"7/8"` */
export function formatSignature(sig: TimeSignature): string {
  return `${sig.numerator}/${sig.denominator}`;
}

/** Parse `"7/8"`; `null` unless numerator is 1..=32 and denominator a power of two ≤ 32. */
export function parseSignature(text: string): TimeSignature | null {
  const m = /^\s*(\d+)\s*\/\s*(\d+)\s*$/.exec(text);
  if (!m) return null;
  const numerator = Number(m[1]);
  const denominator = Number(m[2]);
  if (numerator < 1 || numerator > 32 || !VALID_DENOMINATORS.includes(denominator)) return null;
  return { numerator, denominator };
}

/** `"37%"` */
export function formatCpu(load: number): string {
  return `${Math.round(Math.min(1, Math.max(0, load)) * 100)}%`;
}
