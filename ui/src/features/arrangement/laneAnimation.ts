/**
 * Automation lanes opening/closing in the arrangement (see `features/automation/
 * laneMotion.ts` for the tween itself).
 *
 * While a track's automation slot animates, React lays the rows out once with the slot at
 * the larger of its two heights (`useAutomationSlotHeight`); each animation frame then
 * applies the tweened heights directly, without re-rendering anything:
 *
 * - the row model (`rowsRef`, used by hit testing, marquee and drags) gets the on-screen
 *   rows (`animatedRows`), so clicks land where things are drawn mid-animation;
 * - rows below an animating slot (and the drop area) are shifted up by a transform, which
 *   the compositor applies without re-laying out or repainting their clips;
 * - the animating slot clips its lanes, which slide in/out from its top edge (a transform
 *   too): what shows is exactly the tweened height, and the rows below cover the rest.
 *
 * Rows are stacking contexts at rest (`content-visibility: auto` on `.eth-arr-row`), so the
 * `will-change` of a moving row doesn't change paint order, and rows away from the view are
 * skipped by the browser (no paint, no compositor layer) even while they move (performance
 * audit #14). Moving only the rows near the view was measured too: it costs two more
 * layout passes per toggle (rows entering and leaving the shifted set), for no fewer layers.
 *
 * When the tween ends React re-lays the rows out at their targets, and the next commit
 * clears these styles.
 */

import { useCallback, useEffect, useLayoutEffect, useRef, type MutableRefObject, type RefObject } from "react";
import { animatedSlotHeight, groupFoldProgress, subscribeLaneFrames } from "@/features/automation";
import type { Row } from "./layout";
import { useArrangementUi } from "./uiStore";

/**
 * Rows as on screen mid-animation (pure): each row's automation slot at `slot(row)`, scaled
 * by `fold(row)` (0..1: how unfolded the groups it is in are), the rows below moved
 * accordingly. Rows that don't change are returned as is.
 */
export function animatedRows(rows: ReadonlyArray<Row>, slot: (row: Row) => number, fold: (row: Row) => number = () => 1): Row[] {
  let y = rows[0]?.y ?? 0;
  return rows.map((r) => {
    const f = fold(r);
    const height = (r.laneHeight + slot(r)) * f;
    const out =
      height === r.height && y === r.y
        ? r
        : f === 1
          ? { ...r, y, height }
          : { ...r, y, height, laneHeight: r.laneHeight * f, takesHeight: (r.takesHeight ?? 0) * f, folding: r };
    y += height;
    return out;
  });
}

/**
 * `(row) => 0..1`: how unfolded the groups holding `row` are on screen now (1 when none of
 * them animates). Only groups whose fold animates leave their rows laid out while folded.
 */
function foldNow(rows: ReadonlyArray<Row>): (row: Row) => number {
  const folded = useArrangementUi.getState().folded;
  const byId = new Map(rows.map((r) => [r.track.id, r.track] as const));
  return (row) => {
    let f = 1;
    for (let p = row.track.parent; p; p = byId.get(p)?.parent ?? null) f *= groupFoldProgress(p, !folded.has(p));
    return f;
  };
}

/** Tweened slot height of a laid-out row (its laid-out slot height when not animating). */
const slotNow = (r: Row): number => animatedSlotHeight(r.track.id, r.height - r.laneHeight);

const EPS = 0.01;

function setShift(el: HTMLElement | null | undefined, dy: number): void {
  if (!el) return;
  if (Math.abs(dy) < EPS) {
    if (el.style.transform) {
      el.style.transform = "";
      el.style.willChange = "";
    }
    return;
  }
  // Composited while it moves: each frame is then a transform update only.
  if (!el.style.willChange) el.style.willChange = "transform";
  el.style.transform = `translateY(${dy}px)`;
}

/** Slide the lanes of a row's slot up by `hidden` px (clipped at its top), or reset (`null`). */
function setSlot(slot: HTMLElement | null, hidden: number | null): void {
  if (!slot) return;
  const overflow = hidden === null ? "" : "hidden";
  if (slot.style.overflow !== overflow) slot.style.overflow = overflow;
  setShift(slot.firstElementChild as HTMLElement | null, hidden === null ? 0 : -hidden);
}

/** Cut a row `hidden` px from its bottom, at opacity `alpha`, or reset (`null`). */
function setFold(row: HTMLElement, hidden: number | null, alpha: number): void {
  const clip = hidden === null ? "" : `inset(0 0 ${hidden}px 0)`;
  if (row.style.clipPath !== clip) row.style.clipPath = clip;
  const opacity = hidden === null ? "" : String(alpha);
  if (row.style.opacity !== opacity) row.style.opacity = opacity;
}

/** A rendered row and its automation slot (looked up once per commit, not per frame). */
interface RowEls {
  row: HTMLElement;
  slot: HTMLElement | null;
}

/**
 * Applies the automation lanes' animation frames to the rendered rows (see the module doc).
 * `contentRef`: the element holding the rows (`.eth-arr-row`, keyed by `data-track`) and
 * the drop area; `rows`: the rows as rendered; `rowsRef`: the row model for hit testing.
 */
export function useLaneAnimation(contentRef: RefObject<HTMLElement | null>, rows: ReadonlyArray<Row>, rowsRef: MutableRefObject<ReadonlyArray<Row>>): void {
  const latest = useRef(rows);
  const els = useRef(new Map<string, RowEls>());
  const dropArea = useRef<HTMLElement | null>(null);

  const apply = useCallback(() => {
    const laid = latest.current;
    if (!contentRef.current) return;
    const fold = foldNow(laid);
    const shown = animatedRows(laid, slotNow, fold);
    rowsRef.current = shown;
    for (let i = 0; i < laid.length; i++) {
      const r = laid[i]!;
      const s = shown[i]!;
      const el = els.current.get(r.track.id);
      if (!el) continue;
      setShift(el.row, s.y - r.y);
      const f = fold(r);
      // A row of a group folding / unfolding: cut at its on-screen height (and fading).
      setFold(el.row, f < 1 - EPS ? Math.max(0, r.height - s.height) : null, f);
      if (f >= 1 - EPS) setSlot(el.slot, s.height < r.height - EPS ? r.height - s.height : null);
    }
    const a = laid[laid.length - 1];
    const b = shown[shown.length - 1];
    setShift(dropArea.current, a && b ? b.y + b.height - (a.y + a.height) : 0);
  }, [contentRef, rowsRef]);

  // Every commit: find the row elements, and draw the current frame (or clear) before paint.
  useLayoutEffect(() => {
    latest.current = rows;
    const map = new Map<string, RowEls>();
    const content = contentRef.current;
    if (content) {
      for (const el of content.querySelectorAll<HTMLElement>(":scope > .eth-arr-row[data-track]")) {
        map.set(el.dataset.track!, { row: el, slot: el.querySelector<HTMLElement>(":scope > .eth-arr-row__automation") });
      }
    }
    els.current = map;
    dropArea.current = content?.querySelector<HTMLElement>(":scope > .eth-arr__drop-area") ?? null;
    apply();
  });
  // After the arrangement's own `rowsRef = rows` effect: keep the on-screen rows there.
  useEffect(() => {
    rowsRef.current = animatedRows(rows, slotNow, foldNow(rows));
  });
  useEffect(() => subscribeLaneFrames(apply), [apply]);
}
