/** The adaptive EQ's `Levels` frame (layout in `ether_devices::adaptive_eq::Frame`). */

/** Bands of the device (`adaptive_eq::BANDS`). */
export const ADAPTIVE_EQ_BANDS = 32;

export interface AdaptiveEqReading {
  /** The auto gain now (dB). */
  autoGain: number;
  /** The filters' combined response: frequencies (Hz) and values (dB). */
  freqs: ReadonlyArray<number>;
  response: ReadonlyArray<number>;
  /** Per band: centre (Hz), level relative to the balanced reference (dB), gain (dB). */
  bandFreqs: ReadonlyArray<number>;
  bandLevels: ReadonlyArray<number>;
  bandGains: ReadonlyArray<number>;
}

/** Decode an adaptive EQ `Levels` frame (`null` when malformed). */
export function decodeAdaptiveEq(values: ReadonlyArray<number>): AdaptiveEqReading | null {
  if (values.length < 2) return null;
  const n = Math.max(0, Math.floor(values[0]!));
  const b = ADAPTIVE_EQ_BANDS;
  const at = 2;
  if (values.length < at + 2 * n + 3 * b) return null;
  const s = (from: number, len: number) => values.slice(from, from + len);
  return {
    autoGain: values[1]!,
    freqs: s(at, n),
    response: s(at + n, n),
    bandFreqs: s(at + 2 * n, b),
    bandLevels: s(at + 2 * n + b, b),
    bandGains: s(at + 2 * n + 2 * b, b),
  };
}
