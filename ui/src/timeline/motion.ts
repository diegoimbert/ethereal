/**
 * Physical (spring-based) animation for wheel/keyboard navigation: zoom, pan and scroll.
 *
 * Input moves a *goal*, never the view itself. Each frame, a critically damped spring pulls
 * the current value toward the goal while carrying its velocity, so bursts of wheel events
 * accumulate into one smooth motion and a sudden change of direction bends the motion
 * instead of restarting it. Framerate-independent (exact spring solution per step).
 *
 * Tune the feel in `MOTION`.
 */

/** Motion tuning. Half-lives are in seconds: the time to cover half the remaining distance. */
export const MOTION = {
  /** Off: every change applies instantly (reduced motion, tests). */
  enabled: !(typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches),
  zoomHalfLife: 0.03,
  panHalfLife: 0.03,
  scrollHalfLife: 0.025,
  /** Zoom per pixel of wheel delta (exponential): a 100px mouse notch zooms by e^0.4 ≈ 1.5x. */
  wheelZoomSensitivity: 0.004,
  /** Longest frame step (s): after a stall, don't jump the whole way at once. */
  maxFrameDt: 1 / 30,
};

export interface Spring {
  x: number;
  v: number;
  goal: number;
}

export function spring(x: number): Spring {
  return { x, v: 0, goal: x };
}

/** Advance a critically damped spring by `dt` seconds (exact, stable for any dt). */
export function stepSpring(s: Spring, halfLife: number, dt: number): void {
  const y = (2 * Math.LN2) / Math.max(halfLife, 1e-5);
  const j0 = s.x - s.goal;
  const j1 = s.v + j0 * y;
  const e = Math.exp(-y * dt);
  s.x = e * (j0 + j1 * dt) + s.goal;
  s.v = e * (s.v - j1 * y * dt);
}

/** Jump to the goal and stop. */
export function settle(s: Spring): void {
  s.x = s.goal;
  s.v = 0;
}

/** Whether a spring is at rest, within `eps` (distance) and `eps * 10` per second. */
export function atRest(s: Spring, eps: number): boolean {
  return Math.abs(s.x - s.goal) < eps && Math.abs(s.v) < eps * 10;
}

/**
 * A requestAnimationFrame loop: `tick(dt)` runs every frame while it returns true.
 * `start()` is idempotent, so input handlers can call it on every event.
 */
export function frameLoop(tick: (dt: number) => boolean): { start(): void; stop(): void } {
  let handle: number | null = null;
  let last = 0;
  const frame = (now: number) => {
    const dt = Math.min(MOTION.maxFrameDt, Math.max(0, (now - last) / 1000));
    last = now;
    handle = tick(dt) ? requestAnimationFrame(frame) : null;
  };
  return {
    start() {
      if (handle !== null) return;
      last = performance.now();
      handle = requestAnimationFrame(frame);
    },
    stop() {
      if (handle !== null) cancelAnimationFrame(handle);
      handle = null;
    },
  };
}

/**
 * Animates a multiplicative scale (track heights, key height...) in log space. `push`
 * multiplies the goal; each frame, `apply(factor)` receives the change since the last
 * frame, so the owner keeps its own state (and clamping).
 */
export class ScaleFollower {
  private s = spring(0);
  private readonly loop = frameLoop((dt) => this.frame(dt));

  constructor(private readonly apply: (factor: number) => void) {}

  /**
   * Multiply the goal by `factor`. `bounds` limits the goal's total factor relative to the
   * current (applied) state, so pushing past a limit doesn't store up a dead zone.
   */
  push(factor: number, bounds: { min: number; max: number } = { min: 0, max: Infinity }): void {
    const lo = this.s.x + Math.log(Math.max(bounds.min, 1e-9));
    const hi = this.s.x + Math.log(bounds.max);
    this.s.goal = Math.min(hi, Math.max(lo, this.s.goal + Math.log(factor)));
    if (!MOTION.enabled) {
      const from = this.s.x;
      settle(this.s);
      this.emit(from);
      return;
    }
    this.loop.start();
  }

  cancel(): void {
    this.loop.stop();
    this.s.goal = this.s.x;
    this.s.v = 0;
  }

  private frame(dt: number): boolean {
    const from = this.s.x;
    stepSpring(this.s, MOTION.zoomHalfLife, dt);
    const done = atRest(this.s, 1e-4);
    if (done) settle(this.s);
    this.emit(from);
    return !done;
  }

  private emit(from: number): void {
    if (this.s.x !== from) this.apply(Math.exp(this.s.x - from));
  }
}

/**
 * Animated vertical scrolling of an element. `scrollBy` moves the goal (clamped to the
 * scrollable range). If something else scrolls the element (scrollbar, middle-drag, zoom
 * anchoring), the next input starts from there.
 */
export class ScrollYFollower {
  private s = spring(0);
  private written = NaN;
  private readonly loop = frameLoop((dt) => this.frame(dt));

  constructor(private readonly el: HTMLElement) {}

  scrollBy(dy: number): void {
    const el = this.el;
    if (el.scrollTop !== this.written) {
      // Moved by someone else (or idle): restart from where it is.
      this.s = spring(el.scrollTop);
    }
    const max = Math.max(0, el.scrollHeight - el.clientHeight);
    this.s.goal = Math.min(max, Math.max(0, this.s.goal + dy));
    if (!MOTION.enabled) {
      settle(this.s);
      this.write();
      return;
    }
    this.loop.start();
  }

  cancel(): void {
    this.loop.stop();
    this.written = NaN;
  }

  private frame(dt: number): boolean {
    if (this.el.scrollTop !== this.written && !Number.isNaN(this.written)) {
      // Taken over mid-animation.
      this.written = NaN;
      return false;
    }
    stepSpring(this.s, MOTION.scrollHalfLife, dt);
    const done = atRest(this.s, 0.25);
    if (done) settle(this.s);
    this.write();
    return !done;
  }

  private write(): void {
    this.el.scrollTop = this.s.x;
    // Browsers round scrollTop; remember what it actually became.
    this.written = this.el.scrollTop;
  }
}
