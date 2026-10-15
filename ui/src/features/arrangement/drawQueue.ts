/**
 * Spreads canvas redraws over frames. When the zoom settles, every lane canvas (waveforms,
 * note previews, small clips) must be redrawn at the new zoom: done in one task that was a
 * 100+ ms freeze with a few hundred clips on screen. Until its turn, a canvas keeps showing
 * what it drew before, stretched to the live zoom by its lane layer (`laneGeometry.ts`).
 *
 * `scheduleDraw(key, draw)` queues `draw` for `key` (the canvas; a newer draw replaces a
 * pending one); queued draws run from the next animation frame, a few milliseconds' worth
 * per frame. Redraws for content changes (an edit) are not deferred: they draw at once and
 * `cancelDraw` drops the stale queued one.
 *
 * Work whose cost lands in the browser's rendering rather than in the callback (restyling
 * a lane layer: `--ppb`) passes a `weight` (elements it touches) instead: at most
 * `WEIGHT_BUDGET` of it runs per frame.
 */

type Draw = () => void;

/** Time spent drawing per frame (ms), at least one draw per frame. */
const BUDGET_MS = 6;

/** Elements restyled per frame (weighted entries). */
const WEIGHT_BUDGET = 100;

const queue = new Map<object, { draw: Draw; weight: number }>();
let scheduled = false;

function run(): void {
  scheduled = false;
  const start = performance.now();
  let weight = 0;
  for (const [key, entry] of queue) {
    if (weight > 0 && weight + entry.weight > WEIGHT_BUDGET) continue;
    queue.delete(key);
    entry.draw();
    weight += entry.weight;
    if (performance.now() - start > BUDGET_MS) break;
  }
  if (queue.size > 0) schedule();
}

function schedule(): void {
  if (scheduled) return;
  scheduled = true;
  requestAnimationFrame(run);
}

export function scheduleDraw(key: object, draw: Draw, weight = 0): void {
  queue.set(key, { draw, weight });
  schedule();
}

export function cancelDraw(key: object): void {
  queue.delete(key);
}

/** Run every queued draw now (tests). */
export function flushDraws(): void {
  for (const [key, entry] of queue) {
    queue.delete(key);
    entry.draw();
  }
}
