/**
 * Opening/closing animation of the automation lanes.
 *
 * The arrangement lays its rows out from a model (`layoutRows`), and hit testing and drags
 * use that model, so the animation moves the *layout* height, not just a CSS clip: each
 * time a track's automation height changes (open/close, show/hide a lane), its height is
 * tweened from what is on screen now to the new target over `motion.lane` (a design token),
 * one value per animation frame (`animatedSlotHeight`, `subscribeLaneFrames`).
 *
 * For speed, React only lays the rows out at the start and end of a tween: meanwhile the
 * slot takes the larger of its two heights (`useAutomationSlotHeight`), and the
 * arrangement applies each frame's heights itself (its `laneAnimation.ts`: row transforms,
 * slot clip, and the row model used for hit testing), so clips never re-render per frame.
 *
 * A change during a tween starts a new tween from the current value (no jump). Reduced
 * motion (`MOTION.enabled`, shared with the timeline) applies every change instantly.
 */

import { useCallback, useState } from "react";
import { create } from "zustand";
import type { TrackId } from "@/generated";
import { motion } from "@/theme";
import { MOTION } from "@/timeline/motion";
import { automationHeight, useAutomationHeight, useAutomationUi, type HeightState } from "./uiStore";

/** `"300ms"` / `"0.3s"` → milliseconds. */
export function parseDuration(css: string): number {
  const m = /^\s*(-?[\d.]+)\s*(ms|s)\s*$/.exec(css);
  if (!m) return 0;
  const n = Number(m[1]);
  return m[2] === "s" ? n * 1000 : n;
}

/**
 * Easing function of a CSS `cubic-bezier(x1, y1, x2, y2)` (anything else: linear). Solves
 * x(t) = progress by Newton steps with a bisection fallback, like browsers do.
 */
export function cubicBezier(css: string): (p: number) => number {
  const m = /cubic-bezier\(([^)]*)\)/.exec(css);
  const nums = m ? m[1]!.split(",").map(Number) : [];
  if (nums.length !== 4 || nums.some((n) => !Number.isFinite(n))) return (p) => Math.min(1, Math.max(0, p));
  const [x1, y1, x2, y2] = nums as [number, number, number, number];
  const bez = (a: number, b: number, t: number) => 3 * a * (1 - t) * (1 - t) * t + 3 * b * (1 - t) * t * t + t * t * t;
  const dBez = (a: number, b: number, t: number) => 3 * a * (1 - t) * (1 - t) + 6 * (b - a) * (1 - t) * t + 3 * (1 - b) * t * t;
  return (p) => {
    if (p <= 0) return 0;
    if (p >= 1) return 1;
    let t = p;
    for (let i = 0; i < 8; i++) {
      const err = bez(x1, x2, t) - p;
      if (Math.abs(err) < 1e-6) return bez(y1, y2, t);
      const d = dBez(x1, x2, t);
      if (Math.abs(d) < 1e-6) break;
      t -= err / d;
    }
    let lo = 0;
    let hi = 1;
    t = p;
    for (let i = 0; i < 40; i++) {
      const x = bez(x1, x2, t);
      if (Math.abs(x - p) < 1e-7) break;
      if (x < p) lo = t;
      else hi = t;
      t = (lo + hi) / 2;
    }
    return bez(y1, y2, t);
  };
}

interface Tween {
  from: number;
  to: number;
  start: number;
  /** Value at the last `tick` (what is on screen). */
  value: number;
}

export interface HeightAnimatorOptions {
  /** Tween length (ms). */
  duration: number;
  ease: (p: number) => number;
  /** Off: changes apply instantly (reduced motion). Read at each change. */
  enabled?: () => boolean;
}

/**
 * Per-track height tweens (pure: the caller passes the clock). `retarget` starts a tween
 * when a target changes, `tick(now)` advances all of them, `height(track, target)` is the
 * height to lay out: the tweened value while a tween runs, else the target.
 */
export class HeightAnimator {
  private readonly tweens = new Map<TrackId, Tween>();

  constructor(private readonly opts: HeightAnimatorOptions) {}

  /** The target of `track` changed from `from` to `to` at time `now`. */
  retarget(track: TrackId, from: number, to: number, now: number): void {
    const cur = this.tweens.get(track);
    if (!(this.opts.enabled?.() ?? true) || this.opts.duration <= 0) {
      this.tweens.delete(track);
      return;
    }
    // Mid-tween: continue from what is on screen.
    const start = cur ? cur.value : from;
    if (start === to) {
      this.tweens.delete(track);
      return;
    }
    this.tweens.set(track, { from: start, to, start: now, value: start });
  }

  /** Advance every tween to `now`. Returns the tracks whose tween ended. */
  tick(now: number): TrackId[] {
    const ended: TrackId[] = [];
    for (const [track, tw] of this.tweens) {
      const p = Math.min(1, Math.max(0, (now - tw.start) / this.opts.duration));
      tw.value = tw.from + (tw.to - tw.from) * this.opts.ease(p);
      if (p >= 1) {
        this.tweens.delete(track);
        ended.push(track);
      }
    }
    return ended;
  }

  /** Height to lay `track` out with, given its current target. */
  height(track: TrackId, target: number): number {
    return this.tweens.get(track)?.value ?? target;
  }

  /** Largest height of `track`'s tween (the room its content needs meanwhile), or `null`. */
  extent(track: TrackId): number | null {
    const tw = this.tweens.get(track);
    return tw ? Math.max(tw.from, tw.to) : null;
  }

  isAnimating(track: TrackId): boolean {
    return this.tweens.has(track);
  }

  get active(): boolean {
    return this.tweens.size > 0;
  }

  /** Tracks being animated. */
  tracks(): TrackId[] {
    return [...this.tweens.keys()];
  }

  /** Stop animating `track` (it jumps to its target). */
  cancel(track: TrackId): void {
    this.tweens.delete(track);
  }

  clear(): void {
    this.tweens.clear();
  }
}

/** Tracks whose automation height differs between two UI states, with both heights. */
export function changedHeights(
  prev: HeightState,
  next: HeightState,
): Array<{ track: TrackId; from: number; to: number }> {
  const ids = new Set<TrackId>([...prev.open, ...next.open, ...Object.keys(prev.shown), ...Object.keys(next.shown)] as TrackId[]);
  const out: Array<{ track: TrackId; from: number; to: number }> = [];
  for (const track of ids) {
    const from = automationHeight(prev, track);
    const to = automationHeight(next, track);
    if (from !== to) out.push({ track, from, to });
  }
  return out;
}

// ---- App-wide instance -----------------------------------------------------------------

interface LaneMotionState {
  /** Tracks being animated (changes only when a tween starts or ends). */
  animating: ReadonlySet<TrackId>;
  /** Bumped whenever a tween starts or is retargeted (slot extents may change). */
  version: number;
}

export const useLaneMotion = create<LaneMotionState>()(() => ({ animating: new Set<TrackId>(), version: 0 }));

const frameListeners = new Set<() => void>();

/** Call `cb` after every animation frame of a tween (the heights moved). Returns an unsubscribe. */
export function subscribeLaneFrames(cb: () => void): () => void {
  frameListeners.add(cb);
  return () => {
    frameListeners.delete(cb);
  };
}

/** Slot height of `track` on screen now: tweened, else `target`. */
export function animatedSlotHeight(track: TrackId, target: number): number {
  return laneAnimator.height(track, target);
}

export const laneAnimator = new HeightAnimator({
  duration: parseDuration(motion.lane.duration),
  ease: cubicBezier(motion.lane.ease),
  enabled: () => MOTION.enabled,
});

const now = (): number => (typeof performance !== "undefined" ? performance.now() : Date.now());
let raf: number | null = null;

function publishAnimating(): void {
  const next = laneAnimator.tracks();
  const cur = useLaneMotion.getState().animating;
  if (next.length === cur.size && next.every((t) => cur.has(t))) return;
  useLaneMotion.setState({ animating: new Set(next) });
}

function frame(): void {
  raf = null;
  laneAnimator.tick(now());
  // Listeners draw this frame (tweens that just ended at their targets) before React
  // re-lays the rows out at those targets.
  for (const cb of frameListeners) cb();
  publishAnimating();
  if (laneAnimator.active) schedule();
}

function schedule(): void {
  if (raf !== null) return;
  if (typeof requestAnimationFrame !== "function") {
    // No frames (non-browser): finish at once.
    laneAnimator.tick(Infinity);
    publishAnimating();
    return;
  }
  raf = requestAnimationFrame(frame);
}

useAutomationUi.subscribe((next, prev) => {
  const changes = changedHeights(prev, next);
  if (changes.length === 0) return;
  const t = now();
  // Opening/closing and showing/hiding lanes animate; resizing a lane is direct.
  const structural = prev.open !== next.open || prev.shown !== next.shown;
  for (const c of changes) {
    if (structural) laneAnimator.retarget(c.track, c.from, c.to, t);
    else laneAnimator.cancel(c.track);
  }
  publishAnimating();
  useLaneMotion.setState((s) => ({ version: s.version + 1 }));
  if (laneAnimator.active) schedule();
});

/** Stop every lane animation (tests). */
export function resetLaneMotion(): void {
  if (raf !== null && typeof cancelAnimationFrame === "function") cancelAnimationFrame(raf);
  raf = null;
  laneAnimator.clear();
  useLaneMotion.setState({ animating: new Set(), version: 0 });
}

/** Whether `track`'s automation height is being animated (re-renders on start/end only). */
export function useLaneAnimating(track: TrackId): boolean {
  return useLaneMotion((s) => s.animating.has(track));
}

/**
 * `(track) => height` of the automation slot to *lay out* (the drop-in for
 * `useAutomationHeight()` in the arrangement's `layoutRows`): the target, or while a tween
 * runs the larger of its two heights, so the content has room and React re-lays the rows
 * out only when a tween starts or ends. The heights on screen each frame come from
 * `animatedSlotHeight` (see the arrangement's `laneAnimation.ts`).
 */
export function useAutomationSlotHeight(): (track: TrackId) => number {
  const target = useAutomationHeight();
  const animating = useLaneMotion((s) => s.animating);
  const version = useLaneMotion((s) => s.version);
  return useCallback(
    (track: TrackId) => (animating.has(track) ? Math.max(laneAnimator.extent(track) ?? 0, target(track)) : target(track)),
    // `version`: extents change when a tween is retargeted.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [target, animating, version],
  );
}

// ---- What the lanes component shows while animating -------------------------------------

/**
 * What `<TrackAutomationLanes>` renders: while a track animates, lanes (and the bar) that
 * went away stay mounted until the tween ends, so they can fade/collapse instead of
 * vanishing; new ones are marked to fade in.
 */
export interface ShownLanes {
  /** Bar shown (open, or closing). */
  open: boolean;
  /** Lanes to render, in order (current ones plus leaving ones at their old place). */
  keys: ReadonlyArray<string>;
  /** Lanes that appeared during this animation (fade in). */
  entering: ReadonlySet<string>;
  /** The bar appeared during this animation (the track opened). */
  barEntering: boolean;
  /** Lanes hidden during this animation (collapse out). */
  leaving: ReadonlySet<string>;
  /** The whole track is closing (everything fades out; the row clips it). */
  closing: boolean;
}

const NONE: ReadonlySet<string> = new Set();

export function restingLanes(open: boolean, keys: ReadonlyArray<string>): ShownLanes {
  return { open, keys, entering: NONE, barEntering: false, leaving: NONE, closing: false };
}

/** `next` in order, with the items of `prev` it lacks put back after their old predecessor. */
export function mergeKeys(prev: ReadonlyArray<string>, next: ReadonlyArray<string>): string[] {
  const out = [...next];
  let after: string | null = null;
  for (const k of prev) {
    if (!out.includes(k)) out.splice(after === null ? 0 : out.indexOf(after) + 1, 0, k);
    after = k;
  }
  return out;
}

/**
 * Next `ShownLanes` when the track's open state / shown keys change (pure). `animating`:
 * whether the track's height is being tweened now.
 */
export function nextShownLanes(prev: ShownLanes, open: boolean, keys: ReadonlyArray<string>, animating: boolean): ShownLanes {
  if (!animating) return restingLanes(open, keys);
  if (!open) {
    // Closing: keep what was on screen, fade it out.
    return prev.open ? { ...prev, closing: true, entering: NONE, barEntering: false } : restingLanes(false, keys);
  }
  const wasVisible = prev.open && !prev.closing;
  const visible = new Set(wasVisible ? prev.keys.filter((k) => !prev.leaving.has(k)) : []);
  const entering = new Set(wasVisible ? prev.entering : NONE);
  for (const k of keys) if (!visible.has(k)) entering.add(k);
  const shown = wasVisible ? mergeKeys(prev.keys, keys) : [...keys];
  const leaving = new Set(shown.filter((k) => !keys.includes(k)));
  for (const k of leaving) entering.delete(k);
  return { open: true, keys: shown, entering, barEntering: wasVisible ? prev.barEntering : true, leaving, closing: false };
}

/**
 * `ShownLanes` for a track, following the UI store and the tween (see `nextShownLanes`).
 * Re-renders only when those change, not every frame.
 */
export function useShownLanes(track: TrackId, open: boolean, keys: ReadonlyArray<string>): ShownLanes {
  const animating = useLaneAnimating(track);
  const [state, setState] = useState(() => ({ open, keys, animating, shown: restingLanes(open, keys) }));
  if (state.open !== open || state.keys !== keys || state.animating !== animating) {
    const next = { open, keys, animating, shown: nextShownLanes(state.shown, open, keys, animating) };
    setState(next);
    return next.shown;
  }
  return state.shown;
}
