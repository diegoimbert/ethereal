import { useLayoutEffect } from "react";

/**
 * One shared phase for every blinking edit cursor (the arrangement's insert markers, one per
 * track, and the piano roll's): each marker's CSS blink animation is started at the same
 * timeline time, so markers mounted at different moments (Shift-click on another track, rows
 * scrolled into view) blink together. Placing or moving a marker restarts the phase, so every
 * marker shows right away. With `prefers-reduced-motion` the markers have no animation, so
 * this does nothing.
 */

const live = new Set<Element>();
let epoch: number | null = null;

function now(): number {
  const t = typeof document !== "undefined" ? document.timeline?.currentTime : null;
  return typeof t === "number" ? t : performance.now();
}

/** The shared phase's start (timeline ms), started on first use. */
export function blinkEpoch(): number {
  epoch ??= now();
  return epoch;
}

function align(el: Element): void {
  // jsdom and old engines have no Web Animations on elements.
  if (typeof el.getAnimations !== "function") return;
  const start = blinkEpoch();
  for (const a of el.getAnimations()) a.startTime = start;
}

/** Restart the shared phase now (every live marker shows) and realign all live markers. */
export function restartBlink(): void {
  epoch = now();
  for (const el of live) align(el);
}

/** Ref callback for a blinking marker: aligns its blink to the shared phase while mounted. */
export function blinkRef(el: Element | null): (() => void) | undefined {
  if (!el) return undefined;
  live.add(el);
  align(el);
  return () => {
    live.delete(el);
  };
}

/** Restart the shared phase whenever `key` (the marker's selection) changes; returns `blinkRef`. */
export function useBlinkPhase(key: unknown): typeof blinkRef {
  useLayoutEffect(() => {
    if (key != null) restartBlink();
  }, [key]);
  return blinkRef;
}
