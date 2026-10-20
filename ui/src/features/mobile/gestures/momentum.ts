/**
 * Momentum: the release speed of a pan (`VelocityTracker`, over the last
 * `VELOCITY_WINDOW_MS`) and the fling that continues it with exponential decay (`flingStep`,
 * pure; `startFling` runs it on animation frames).
 */
import { FLING_MIN_SPEED, FLING_STOP_SPEED, FLING_TIME_CONSTANT_MS, VELOCITY_WINDOW_MS } from "./constants";
import type { Point } from "./recognizer";

export class VelocityTracker {
  private samples: Array<{ x: number; y: number; t: number }> = [];

  reset(p: Point, t: number): void {
    this.samples = [{ x: p.x, y: p.y, t }];
  }

  add(p: Point, t: number): void {
    this.samples.push({ x: p.x, y: p.y, t });
    const from = t - VELOCITY_WINDOW_MS * 2;
    while (this.samples.length > 2 && this.samples[0]!.t < from) this.samples.shift();
  }

  /** px/ms over the window before `now` (0 when the finger rested before lifting). */
  speed(now: number): Point {
    const recent = this.samples.filter((s) => s.t >= now - VELOCITY_WINDOW_MS);
    if (recent.length < 2) return { x: 0, y: 0 };
    const a = recent[0]!;
    const b = recent[recent.length - 1]!;
    const dt = b.t - a.t;
    if (dt <= 0) return { x: 0, y: 0 };
    return { x: (b.x - a.x) / dt, y: (b.y - a.y) / dt };
  }
}

/** One frame of a fling: the distance to move and the speed left, after `dt` ms at speed `v` (px/ms). */
export function flingStep(v: number, dt: number): { delta: number; v: number } {
  const decay = Math.exp(-dt / FLING_TIME_CONSTANT_MS);
  // The distance covered over `dt` by v(t) = v·e^(-t/τ): v·τ·(1 - e^(-dt/τ)).
  const delta = v * FLING_TIME_CONSTANT_MS * (1 - decay);
  const next = v * decay;
  return { delta, v: Math.abs(next) < FLING_STOP_SPEED ? 0 : next };
}

/** The total distance a fling at speed `v` travels (for tests and scroll limits). */
export const flingDistance = (v: number): number => v * FLING_TIME_CONSTANT_MS;

export const shouldFling = (vx: number, vy: number): boolean => Math.hypot(vx, vy) >= FLING_MIN_SPEED;

/**
 * Continue a pan at (`vx`, `vy`) px/ms, decaying, calling `apply(dx, dy)` each frame. Returns
 * a stop function (a new touch stops the fling at once).
 */
export function startFling(
  vx: number,
  vy: number,
  apply: (dx: number, dy: number) => void,
  raf = requestAnimationFrame,
  caf = cancelAnimationFrame,
): () => void {
  if (!shouldFling(vx, vy)) return () => undefined;
  let handle = 0;
  let last: number | null = null;
  let sx = vx;
  let sy = vy;
  const frame = (now: number) => {
    const dt = last === null ? 16 : Math.min(64, now - last);
    last = now;
    const x = flingStep(sx, dt);
    const y = flingStep(sy, dt);
    sx = x.v;
    sy = y.v;
    apply(x.delta, y.delta);
    if (sx !== 0 || sy !== 0) handle = raf(frame);
  };
  handle = raf(frame);
  return () => caf(handle);
}
