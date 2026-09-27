/**
 * Animated zoom/pan of a timeline view store (see `motion.ts`).
 *
 * Two springs: log(pxPerBeat), and the beat under an anchor x (the pointer). Scroll is
 * derived from them, so a zoom stays locked on the beat under the pointer for the whole
 * animation, and pans and zooms mix freely. When the anchor moves, the springs are
 * re-expressed at the new anchor keeping the scroll and its velocity continuous.
 *
 * Anything else that changes the view (ruler drag, playhead follow, middle-button pan)
 * takes over: the animation stops and the next input starts from the new state.
 */

import { atRest, frameLoop, MOTION, settle, spring, stepSpring, type Spring } from "./motion";
import type { TimelineViewStore } from "./viewStore";

class ViewMotion {
  /** log(pxPerBeat). */
  private zoom: Spring = spring(0);
  /** Beat at `anchorX`. */
  private beat: Spring = spring(0);
  private anchorX = 0;
  private written: { pxPerBeat: number; scrollBeats: number } | null = null;
  private readonly loop = frameLoop((dt) => this.frame(dt));

  constructor(private readonly view: TimelineViewStore) {}

  zoomBy(factor: number, anchorPx: number): void {
    this.sync();
    this.reanchor(anchorPx);
    const { minPxPerBeat, maxPxPerBeat } = this.view.getState().limits;
    this.zoom.goal = Math.min(Math.log(maxPxPerBeat), Math.max(Math.log(minPxPerBeat), this.zoom.goal + Math.log(factor)));
    this.clampGoal();
    this.run();
  }

  panByPx(dx: number): void {
    this.sync();
    this.beat.goal += dx / Math.exp(this.zoom.goal);
    this.clampGoal();
    this.run();
  }

  /** Adopt the store's state unless it is what this animation last wrote. */
  private sync(): void {
    const s = this.view.getState();
    const w = this.written;
    if (w && w.pxPerBeat === s.pxPerBeat && w.scrollBeats === s.scrollBeats) return;
    this.loop.stop();
    this.zoom = spring(Math.log(s.pxPerBeat));
    this.beat = spring(s.scrollBeats + this.anchorX / s.pxPerBeat);
    this.written = { pxPerBeat: s.pxPerBeat, scrollBeats: s.scrollBeats };
  }

  /** Move the anchor to `x` keeping scroll (value, velocity and goal) unchanged. */
  private reanchor(x: number): void {
    const X = this.anchorX;
    if (x === X) return;
    const p = Math.exp(this.zoom.x);
    const pGoal = Math.exp(this.zoom.goal);
    // scroll = beat - X / p  →  d(scroll)/dt = beat.v + X * zoom.v / p
    const scroll = this.beat.x - X / p;
    const scrollV = this.beat.v + (X * this.zoom.v) / p;
    const scrollGoal = this.beat.goal - X / pGoal;
    this.beat = { x: scroll + x / p, v: scrollV - (x * this.zoom.v) / p, goal: scrollGoal + x / pGoal };
    this.anchorX = x;
  }

  /** Keep the goal's scroll at or after `minScrollBeats`. */
  private clampGoal(): void {
    const min = this.view.getState().limits.minScrollBeats;
    const pGoal = Math.exp(this.zoom.goal);
    if (this.beat.goal - this.anchorX / pGoal < min) this.beat.goal = min + this.anchorX / pGoal;
  }

  private run(): void {
    if (!MOTION.enabled) {
      settle(this.zoom);
      settle(this.beat);
      this.write();
      return;
    }
    this.loop.start();
  }

  private frame(dt: number): boolean {
    const s = this.view.getState();
    const w = this.written;
    if (!w || w.pxPerBeat !== s.pxPerBeat || w.scrollBeats !== s.scrollBeats) return false; // taken over
    stepSpring(this.zoom, MOTION.zoomHalfLife, dt);
    stepSpring(this.beat, MOTION.panHalfLife, dt);
    const p = Math.exp(this.zoom.x);
    // Rest thresholds: ~1e-4 relative zoom, ~0.1 px of scroll.
    const done = atRest(this.zoom, 1e-4) && atRest(this.beat, 0.1 / p);
    if (done) {
      settle(this.zoom);
      settle(this.beat);
    }
    this.write();
    return !done;
  }

  private write(): void {
    const p = Math.exp(this.zoom.x);
    this.view.getState().setViewport({ pxPerBeat: p, scrollBeats: this.beat.x - this.anchorX / p });
    const s = this.view.getState();
    this.written = { pxPerBeat: s.pxPerBeat, scrollBeats: s.scrollBeats };
  }
}

const motions = new WeakMap<TimelineViewStore, ViewMotion>();

function motionOf(view: TimelineViewStore): ViewMotion {
  let m = motions.get(view);
  if (!m) motions.set(view, (m = new ViewMotion(view)));
  return m;
}

/** Animated zoom by `factor` around `anchorPx` (default: the view center). Bursts accumulate. */
export function animateZoom(view: TimelineViewStore, factor: number, anchorPx?: number): void {
  motionOf(view).zoomBy(factor, anchorPx ?? view.getState().widthPx / 2);
}

/** Animated horizontal scroll by `dx` px (positive = later in time). Bursts accumulate. */
export function animatePan(view: TimelineViewStore, dx: number): void {
  motionOf(view).panByPx(dx);
}
