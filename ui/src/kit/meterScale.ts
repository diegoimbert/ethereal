/** Maps linear amplitude (1.0 = 0 dBFS) to a 0..1 meter position on a dB scale. */
export function meterPosition(linear: number, minDb = -60, maxDb = 6): number {
  if (!(linear > 0)) return 0;
  const db = 20 * Math.log10(linear);
  return Math.min(1, Math.max(0, (db - minDb) / (maxDb - minDb)));
}
