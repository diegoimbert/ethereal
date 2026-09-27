/**
 * Horizontal viewport math: beats ↔ pixels, seconds ↔ pixels (through a `TempoMap`),
 * zoom around an anchor and clamping. All pure; `viewStore.ts` wraps it in a store.
 *
 * Pixel coordinates are relative to the left edge of the timeline's content area.
 */

import type { BeatRange, Beats, Seconds } from "@/generated";
import { MOTION } from "./motion";
import type { TempoMap } from "./tempoMap";

/** Horizontal viewport of a timeline. */
export interface TimelineViewport {
  /** Zoom: pixels per beat. */
  pxPerBeat: number;
  /** Beat at the left edge of the viewport. */
  scrollBeats: number;
}

export interface ZoomLimits {
  minPxPerBeat: number;
  maxPxPerBeat: number;
  /** Smallest allowed `scrollBeats` (default 0: nothing before the song start). */
  minScrollBeats: number;
}

export const DEFAULT_ZOOM_LIMITS: ZoomLimits = { minPxPerBeat: 0.25, maxPxPerBeat: 2000, minScrollBeats: 0 };

/** Beat position → x pixel within the viewport. */
export function beatsToPx(beats: Beats, vp: TimelineViewport): number {
  return (beats - vp.scrollBeats) * vp.pxPerBeat;
}

/** x pixel within the viewport → beat position. */
export function pxToBeats(px: number, vp: TimelineViewport): Beats {
  return px / vp.pxPerBeat + vp.scrollBeats;
}

/** A beat length → pixel width. */
export function beatsToWidth(beats: Beats, vp: TimelineViewport): number {
  return beats * vp.pxPerBeat;
}

/** A pixel width → beat length. */
export function widthToBeats(px: number, vp: TimelineViewport): Beats {
  return px / vp.pxPerBeat;
}

/** Seconds (from song start) → x pixel, through the tempo map. */
export function secondsToPx(seconds: Seconds, vp: TimelineViewport, tempo: TempoMap): number {
  return beatsToPx(tempo.secondsToBeats(seconds), vp);
}

/** x pixel → seconds (from song start), through the tempo map. */
export function pxToSeconds(px: number, vp: TimelineViewport, tempo: TempoMap): Seconds {
  return tempo.beatsToSeconds(pxToBeats(px, vp));
}

/** Beat range visible in a viewport `widthPx` wide. */
export function visibleRange(vp: TimelineViewport, widthPx: number): BeatRange {
  return { start: vp.scrollBeats, end: vp.scrollBeats + widthPx / vp.pxPerBeat };
}

/** Clamp zoom and scroll to `limits`. */
export function clampViewport(vp: TimelineViewport, limits: ZoomLimits = DEFAULT_ZOOM_LIMITS): TimelineViewport {
  const pxPerBeat = Math.min(limits.maxPxPerBeat, Math.max(limits.minPxPerBeat, vp.pxPerBeat));
  const scrollBeats = Math.max(limits.minScrollBeats, vp.scrollBeats);
  return pxPerBeat === vp.pxPerBeat && scrollBeats === vp.scrollBeats ? vp : { pxPerBeat, scrollBeats };
}

/**
 * Set the zoom to `pxPerBeat` keeping the beat under `anchorPx` fixed (zoom around the
 * mouse cursor). The result is clamped.
 */
export function zoomTo(
  vp: TimelineViewport,
  pxPerBeat: number,
  anchorPx: number,
  limits: ZoomLimits = DEFAULT_ZOOM_LIMITS,
): TimelineViewport {
  const anchorBeats = pxToBeats(anchorPx, vp);
  const z = Math.min(limits.maxPxPerBeat, Math.max(limits.minPxPerBeat, pxPerBeat));
  return clampViewport({ pxPerBeat: z, scrollBeats: anchorBeats - anchorPx / z }, limits);
}

/** Multiply the zoom by `factor` around `anchorPx` (factor > 1 zooms in). */
export function zoomBy(
  vp: TimelineViewport,
  factor: number,
  anchorPx: number,
  limits: ZoomLimits = DEFAULT_ZOOM_LIMITS,
): TimelineViewport {
  return zoomTo(vp, vp.pxPerBeat * factor, anchorPx, limits);
}

/** Fit `range` into `widthPx` (with `paddingPx` on each side). */
export function zoomToRange(
  range: BeatRange,
  widthPx: number,
  paddingPx = 0,
  limits: ZoomLimits = DEFAULT_ZOOM_LIMITS,
): TimelineViewport {
  const len = Math.max(range.end - range.start, 1e-3);
  const usable = Math.max(1, widthPx - 2 * paddingPx);
  const pxPerBeat = Math.min(limits.maxPxPerBeat, Math.max(limits.minPxPerBeat, usable / len));
  return clampViewport({ pxPerBeat, scrollBeats: range.start - paddingPx / pxPerBeat }, limits);
}

/**
 * Scroll so that `beats` is visible, keeping `marginPx` from either edge. Returns `vp`
 * unchanged when it already is (use for "follow playhead": pages when the playhead
 * leaves the view).
 */
export function revealBeats(
  vp: TimelineViewport,
  beats: Beats,
  widthPx: number,
  marginPx = 0,
  limits: ZoomLimits = DEFAULT_ZOOM_LIMITS,
): TimelineViewport {
  const x = beatsToPx(beats, vp);
  const margin = Math.min(marginPx, widthPx / 2);
  if (x >= margin && x <= widthPx - margin) return vp;
  const scrollBeats = x < margin ? beats - margin / vp.pxPerBeat : beats - (widthPx - margin) / vp.pxPerBeat;
  return clampViewport({ ...vp, scrollBeats }, limits);
}

/** Zoom factor for a wheel `deltaY` (pixels): smooth, exponential, sign-correct. */
export function wheelZoomFactor(deltaY: number, sensitivity = MOTION.wheelZoomSensitivity): number {
  return Math.exp(-deltaY * sensitivity);
}
