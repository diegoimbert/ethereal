/**
 * Fine control on touch screens (the design's §4 "Faders, knobs, numbers"), like iOS's
 * scrubbing sliders: there is no Shift key, so moving the finger away from the control's axis
 * slows the drag down. Found by accident, nothing to learn.
 */

/** Off-axis distance (px) where each slower rate starts, and the rates. */
const STEPS: ReadonlyArray<readonly [number, number]> = [
  [180, 0.1],
  [120, 0.25],
  [60, 0.5],
];

/** The drag rate for a finger `offAxis` px away from where it started, across the control's axis. */
export function touchFineFactor(offAxis: number): number {
  const d = Math.abs(offAxis);
  for (const [from, rate] of STEPS) if (d >= from) return rate;
  return 1;
}
