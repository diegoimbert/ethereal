import clsx from "clsx";
import { meterPosition } from "./meterScale";

export interface MeterProps {
  /** Linear peak amplitude per channel (1.0 = 0 dBFS; values above 1 clip). */
  levels: number | readonly number[];
  /** Pixel height for one-offs; default is the `--meter-height` token. */
  height?: number;
  /** Bottom of the scale in dB. */
  minDb?: number;
  /** Top of the scale in dB (headroom above 0 dBFS). */
  maxDb?: number;
  className?: string;
}

/**
 * Vertical peak meter (dB scale). Pure display: the caller feeds it values at visual rate.
 * Colors/stops/widths come from `--meter-*` component tokens.
 */
export function Meter({ levels, height, minDb = -60, maxDb = 6, className }: MeterProps) {
  const channels = typeof levels === "number" ? [levels] : levels;
  return (
    <div className={clsx("eth-meter", className)} style={height !== undefined ? { height } : undefined} role="meter" aria-valuemin={0} aria-valuemax={1}>
      {channels.map((lin, i) => (
        <div key={i} className="eth-meter__channel">
          <div className="eth-meter__gradient" />
          <div className="eth-meter__mask" style={{ height: `${(1 - meterPosition(lin, minDb, maxDb)) * 100}%` }} />
          {lin > 1 && <div className="eth-meter__clip" />}
        </div>
      ))}
    </div>
  );
}
