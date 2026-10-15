/**
 * The pixel ratio the timeline canvases (the arranger, the ruler, the piano roll) render at: the
 * device's, capped on mobile (#522, the design's §11): a DPR 3 phone would fill 2.25× the pixels
 * of DPR 2 for no visible gain. The mobile UI sets the cap (`setRenderDprCap`); the desktop has
 * none.
 */
let cap = Infinity;

/** Caps the timeline canvases' pixel ratio (`Infinity`: no cap). */
export function setRenderDprCap(max: number): void {
  cap = max;
}

/** The device pixel ratio, capped. */
export function renderDpr(): number {
  const dpr = typeof window === "undefined" ? 1 : window.devicePixelRatio || 1;
  return Math.min(dpr, cap);
}
