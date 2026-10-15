/**
 * Lane geometry that follows zoom and scroll without React.
 *
 * Lane content (clips, previews, markers of a lane) is positioned in beats with CSS:
 * `x = (beats - origin) * var(--ppb)`, where `LaneLayer` writes the *settled* zoom (below)
 * into `--ppb`, and the scroll plus the live/settled zoom ratio into its transform
 * (`translateX(…) scaleX(live / settled)`). So an animated pan or zoom only changes one
 * transform per lane per frame, composited on the GPU: no style recalc of the hundreds of
 * clips inside (writing `--ppb` every frame restyled every element under the lane). The
 * content is stretched by at most `MAX_DRIFT` until the zoom settles and `--ppb` follows.
 *
 * Components still need a pixels-per-beat for pixel math (canvases, drag deltas):
 * `useSettledZoom` gives the zoom React renders with. It follows the live zoom once it has
 * been still for `SETTLE_MS`, or at once when the live zoom drifts past `MAX_DRIFT` (so
 * stretched canvases never get too blurry). At rest it is exact.
 */

import type { Beats } from "@/generated";

export { MAX_DRIFT, SETTLE_MS, settledZoom, subscribeSettledZoom, useSettledZoom } from "@/timeline";

/** CSS length of a beat offset in a lane layer: `offset * var(--ppb)`. */
export function beatsCss(beats: Beats): string {
  return `calc(${beats} * var(--ppb))`;
}

/** CSS width of a beat length, at least `minPx` wide. */
export function widthCss(beats: Beats, minPx = 0): string {
  return minPx > 0 ? `max(${minPx}px, calc(${beats} * var(--ppb)))` : beatsCss(beats);
}
