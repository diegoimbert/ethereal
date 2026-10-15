// The mounted arranger renderer (WebGL or its Canvas2D fallback), if any (for the test/dev API and phase-2 layers that
// mount outside the arrangement's React tree).
import type { ArrangerRenderer } from "./renderer";

let current: ArrangerRenderer | null = null;
const listeners = new Set<(r: ArrangerRenderer | null) => void>();

export function setGlRenderer(r: ArrangerRenderer | null): void {
  current = r;
  for (const l of listeners) l(r);
}

export function glRenderer(): ArrangerRenderer | null {
  return current;
}

/** Calls `fn` with the renderer now and whenever it changes (mount, unmount). */
export function onGlRenderer(fn: (r: ArrangerRenderer | null) => void): () => void {
  listeners.add(fn);
  fn(current);
  return () => listeners.delete(fn);
}
