/**
 * Per-track meter reading for a strip, fed by the high-rate meter stream
 * (`playheadStore`). Tracks missing from a frame keep their last reading, so after a short
 * hold the display decays on its own; the clip indicator latches until reset.
 */

import { useCallback, useEffect, useState } from "react";
import type { TrackId } from "@/generated";
import { playheadStore } from "@/state";

/** Hold time before a stale reading starts falling. */
export const METER_HOLD_MS = 100;
/** Fall rate of a stale reading. */
export const METER_DECAY_DB_PER_S = 30;
const TICK_MS = 33;
const FLOOR = 1e-4; // -80 dB

export interface MeterReading {
  levels: [number, number];
  clipped: boolean;
}

const ZERO: [number, number] = [0, 0];

/** Linear level after decaying for `ageMs` (hold, then `METER_DECAY_DB_PER_S`). */
export function decayed(level: number, ageMs: number): number {
  if (ageMs <= METER_HOLD_MS) return level;
  const v = level * Math.pow(10, (-METER_DECAY_DB_PER_S * (ageMs - METER_HOLD_MS)) / 1000 / 20);
  return v < FLOOR ? 0 : v;
}

export function useMeterLevels(track: TrackId): MeterReading & { resetClip(): void } {
  const [reading, setReading] = useState<MeterReading>(() => {
    const m = playheadStore.getMeter(track);
    return { levels: m ? m.peak : ZERO, clipped: m?.clipped ?? false };
  });

  useEffect(() => {
    let last: { peak: [number, number]; at: number } | null = null;
    const unsubscribe = playheadStore.subscribeMeter(track, () => {
      const m = playheadStore.getMeter(track);
      if (!m) {
        last = null;
        setReading((r) => ({ levels: ZERO, clipped: r.clipped }));
        return;
      }
      last = { peak: m.peak, at: performance.now() };
      setReading((r) => ({ levels: m.peak, clipped: r.clipped || m.clipped }));
    });
    const timer = setInterval(() => {
      if (!last) return;
      const age = performance.now() - last.at;
      if (age <= METER_HOLD_MS) return;
      const levels: [number, number] = [decayed(last.peak[0], age), decayed(last.peak[1], age)];
      if (levels[0] === 0 && levels[1] === 0) last = null;
      setReading((r) => ({ levels, clipped: r.clipped }));
    }, TICK_MS);
    return () => {
      unsubscribe();
      clearInterval(timer);
    };
  }, [track]);

  const resetClip = useCallback(() => setReading((r) => ({ ...r, clipped: false })), []);
  return { ...reading, resetClip };
}
