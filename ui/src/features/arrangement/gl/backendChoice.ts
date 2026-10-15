/**
 * Which backend draws the arranger: WebGL2, or the Canvas2D fallback (`canvas2d.ts`) when
 * this browser has no working WebGL2 (`webgl2Supported()` false, the context can't be
 * created twice in a row, or it is lost and not given back). Automatic: there is no user
 * setting. A hidden debug override forces the fallback (tests, debugging):
 * `localStorage["eth.arr.renderer"] = "canvas2d"` (read at startup), or
 * `window.__ether.setArrangerRenderer("canvas2d" | "auto")` at runtime.
 */

import { create } from "zustand";
import { webgl2Supported } from "./support";

export type BackendKind = "webgl" | "canvas2d";
export type RendererOverride = "auto" | "canvas2d";

export const RENDERER_OVERRIDE_KEY = "eth.arr.renderer";
/** WebGL failures (context creation, lost and not restored) before falling back for the session. */
const MAX_WEBGL_FAILURES = 2;
/** Context losses (even restored ones) before falling back for the session. */
const MAX_WEBGL_LOSSES = 3;
/** A lost context not restored within this long counts as a failure (ms). */
export const WEBGL_RESTORE_TIMEOUT_MS = 3000;

function storedOverride(): RendererOverride {
  try {
    return localStorage.getItem(RENDERER_OVERRIDE_KEY) === "canvas2d" ? "canvas2d" : "auto";
  } catch {
    return "auto";
  }
}

interface BackendState {
  override: RendererOverride;
  /** WebGL failures this session. */
  failures: number;
  /** WebGL context losses this session. */
  losses: number;
  /** Bumped on each failure: the arranger remounts its canvas (a retry, or the fallback). */
  attempt: number;
}

export const useArrangerBackend = create<BackendState>()(() => ({ override: storedOverride(), failures: 0, losses: 0, attempt: 0 }));

/** The backend to draw with now. */
export function backendKind(s: BackendState = useArrangerBackend.getState()): BackendKind {
  if (s.override === "canvas2d" || s.failures >= MAX_WEBGL_FAILURES || !webgl2Supported()) return "canvas2d";
  return "webgl";
}

/** The debug override (persisted like the old setting, so a reload keeps it). */
export function setRendererOverride(v: RendererOverride | "webgl"): void {
  const override: RendererOverride = v === "canvas2d" ? "canvas2d" : "auto";
  try {
    if (override === "canvas2d") localStorage.setItem(RENDERER_OVERRIDE_KEY, override);
    else localStorage.removeItem(RENDERER_OVERRIDE_KEY);
  } catch {
    /* not persisted */
  }
  useArrangerBackend.setState({ override });
}

/**
 * WebGL failed (the context couldn't be created, or was lost and not restored): the
 * arranger remounts its canvas, on WebGL again the first time, then on Canvas2D.
 */
export function reportWebglFailure(): void {
  useArrangerBackend.setState((s) => ({ failures: s.failures + 1, attempt: s.attempt + 1 }));
}

/**
 * The WebGL context was lost (the renderer restores it when the browser gives it back): a
 * context lost again and again means a GPU that can't keep it, so the arranger falls back.
 */
export function reportWebglLoss(): void {
  useArrangerBackend.setState((s) => {
    const losses = s.losses + 1;
    return losses >= MAX_WEBGL_LOSSES ? { losses, failures: MAX_WEBGL_FAILURES, attempt: s.attempt + 1 } : { losses };
  });
}
