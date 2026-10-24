/** The pitch correction's `Levels` frame (layout in `ether_devices::pitch_correction::Frame`). */

/** Indices of the frame's values (mirrors `pitch_correction::Frame`). */
export const PITCH_CORRECTION_FRAME = {
  detected: 0,
  output: 1,
  target: 2,
  correction: 3,
  confidence: 4,
  step: 5,
  points: 6,
  history: 7,
} as const;

export interface PitchCorrectionReading {
  /** MIDI notes (fractional), -1 when unvoiced. */
  detected: number;
  output: number;
  /** The target note, -1 when none. */
  target: number;
  /** Cents (output − detected). */
  correction: number;
  confidence: number;
  /** Seconds per history point. */
  step: number;
  detectedHistory: ReadonlyArray<number>;
  outputHistory: ReadonlyArray<number>;
}

/** Decode a pitch correction `Levels` frame (`null` when malformed). */
export function decodePitchCorrection(values: ReadonlyArray<number>): PitchCorrectionReading | null {
  const f = PITCH_CORRECTION_FRAME;
  if (values.length < f.history) return null;
  const n = Math.max(0, Math.floor(values[f.points]!));
  if (values.length < f.history + 2 * n) return null;
  return {
    detected: values[f.detected]!,
    output: values[f.output]!,
    target: values[f.target]!,
    correction: values[f.correction]!,
    confidence: values[f.confidence]!,
    step: values[f.step]!,
    detectedHistory: values.slice(f.history, f.history + n),
    outputHistory: values.slice(f.history + n, f.history + 2 * n),
  };
}

/** Polyline point lists of a pitch series (one list per voiced run), `x(i)` / `y(note)` in px. */
export function pitchRuns(series: ReadonlyArray<number>, x: (i: number) => number, y: (note: number) => number): string[] {
  const runs: string[] = [];
  let cur: string[] = [];
  series.forEach((v, i) => {
    if (v < 0) {
      if (cur.length > 1) runs.push(cur.join(" "));
      cur = [];
      return;
    }
    cur.push(`${x(i).toFixed(1)},${y(v).toFixed(1)}`);
  });
  if (cur.length > 1) runs.push(cur.join(" "));
  return runs;
}

const NAMES = ["C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B"];

/** "A3" for 57 (the tuner's naming: middle C = C4). */
export function pitchName(note: number): string {
  const n = Math.round(note);
  return `${NAMES[((n % 12) + 12) % 12]}${Math.floor(n / 12) - 1}`;
}
