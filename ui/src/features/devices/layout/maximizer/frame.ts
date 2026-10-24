/** The maximizer's `Levels` frame (layout in `ether_devices::maximizer::Frame`). */

/** Indices of the frame's values (mirrors `maximizer::Frame`). */
export const MAXIMIZER_FRAME = {
  gainReduction: 0,
  input: 1,
  output: 2,
  outputMax: 3,
  gainReductionMax: 4,
  step: 5,
  points: 6,
  history: 7,
} as const;

export interface MaximizerReading {
  /** Deepest gain reduction since the previous frame (dB, <= 0). */
  gainReduction: number;
  /** Loudest input (before the threshold gain) and output true peaks since the previous frame (dB). */
  input: number;
  output: number;
  /** Loudest output true peak and deepest gain reduction since the reset. */
  outputMax: number;
  gainReductionMax: number;
  /** Seconds per history point. */
  step: number;
  inputHistory: ReadonlyArray<number>;
  outputHistory: ReadonlyArray<number>;
  gainReductionHistory: ReadonlyArray<number>;
}

/** Decode a maximizer `Levels` frame (`null` when malformed). */
export function decodeMaximizer(values: ReadonlyArray<number>): MaximizerReading | null {
  const f = MAXIMIZER_FRAME;
  if (values.length < f.history) return null;
  const n = Math.max(0, Math.floor(values[f.points]!));
  if (values.length < f.history + 3 * n) return null;
  const series = (k: number) => values.slice(f.history + k * n, f.history + (k + 1) * n);
  return {
    gainReduction: values[f.gainReduction]!,
    input: values[f.input]!,
    output: values[f.output]!,
    outputMax: values[f.outputMax]!,
    gainReductionMax: values[f.gainReductionMax]!,
    step: values[f.step]!,
    inputHistory: series(0),
    outputHistory: series(1),
    gainReductionHistory: series(2),
  };
}
