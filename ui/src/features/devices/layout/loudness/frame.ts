/** The loudness meter's `Levels` frame (layout in `ether_devices::fx_analysis::loudness_device::Frame`). */

/** Indices of the frame's values (mirrors `loudness_device::Frame`). */
export const LOUDNESS_FRAME = {
  momentary: 0,
  shortTerm: 1,
  integrated: 2,
  range: 3,
  truePeakMax: 4,
  truePeak: 5,
  window: 6,
  step: 7,
  points: 8,
  history: 9,
} as const;

export interface LoudnessReading {
  momentary: number;
  shortTerm: number;
  integrated: number;
  range: number;
  truePeakMax: number;
  truePeak: number;
  window: number;
  step: number;
  momentaryHistory: ReadonlyArray<number>;
  shortTermHistory: ReadonlyArray<number>;
}

/** Decode a loudness `Levels` frame (`null` when malformed). */
export function decodeLoudness(values: ReadonlyArray<number>): LoudnessReading | null {
  const f = LOUDNESS_FRAME;
  if (values.length < f.history) return null;
  const n = Math.max(0, Math.floor(values[f.points]!));
  if (values.length < f.history + 2 * n) return null;
  return {
    momentary: values[f.momentary]!,
    shortTerm: values[f.shortTerm]!,
    integrated: values[f.integrated]!,
    range: values[f.range]!,
    truePeakMax: values[f.truePeakMax]!,
    truePeak: values[f.truePeak]!,
    window: values[f.window]!,
    step: values[f.step]!,
    momentaryHistory: values.slice(f.history, f.history + n),
    shortTermHistory: values.slice(f.history + n, f.history + 2 * n),
  };
}
