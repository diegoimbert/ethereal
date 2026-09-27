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
 * When the tween ends React re-lays the rows out at their targets, and the next commit
 * clears these styles.
 */

import { useCallback, useEffect, useLayoutEffect, useRef, type MutableRefObject, type RefObject } from "react";
import { animatedSlotHeight, subscribeLaneFrames } from "@/features/automation";
import type { Row } from "./layout";

/**
 * Rows as on screen mid-animation (pure): each row's automation slot at `slot(row)`, the
 * rows below moved accordingly. Rows that don't change are returned as is.
 */
export function animatedRows(rows: ReadonlyArray<Row>, slot: (row: Row) => number): Row[] {
  let y = rows[0]?.y ?? 0;
  return rows.map((r) => {
    const height = r.draft ? r.height : r.laneHeight + slot(r);
    const out = height === r.height && y === r.y ? r : { ...r, y, height };
    y += height;
    return out;
  });
}

/** Tweened slot height of a laid-out row (its laid-out slot height when not animating). */
const slotNow = (r: Row): number => (r.draft ? 0 : animatedSlotHeight(r.track.id, r.height - r.laneHeight));

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
  el.style.willChange = "transform";
  el.style.transform = `translateY(${dy}px)`;
}

/** Slide the lanes of a row's slot up by `hidden` px (clipped at its top), or reset (`null`). */
function setSlot(row: HTMLElement | undefined, hidden: number | null): void {
  const slot = row?.querySelector<HTMLElement>(":scope > .eth-arr-row__automation");
  if (!slot) return;
  slot.style.overflow = hidden === null ? "" : "hidden";
  setShift(slot.firstElementChild as HTMLElement | null, hidden === null ? 0 : -hidden);
}

/**
 * Applies the automation lanes' animation frames to the rendered rows (see the module doc).
 * `contentRef`: the element holding the rows (`.eth-arr-row`, keyed by `data-track`) and
 * the drop area; `rows`: the rows as rendered; `rowsRef`: the row model for hit testing.
 */
export function useLaneAnimation(
  contentRef: RefObject<HTMLElement | null>,
  rows: ReadonlyArray<Row>,
  rowsRef: MutableRefObject<ReadonlyArray<Row>>,
): void {
  const latest = useRef(rows);
  const els = useRef(new Map<string, HTMLElement>());

  const apply = useCallback(() => {
    const content = contentRef.current;
    const laid = latest.current;
    if (!content) return;
    const shown = animatedRows(laid, slotNow);
    rowsRef.current = shown;
    for (let i = 0; i < laid.length; i++) {
      const r = laid[i]!;
      const s = shown[i]!;
      const el = els.current.get(r.track.id);
      setShift(el, s.y - r.y);
      setSlot(el, s.height < r.height - EPS ? r.height - s.height : null);
    }
    const a = laid[laid.length - 1];
    const b = shown[shown.length - 1];
    setShift(content.querySelector<HTMLElement>(":scope > .eth-arr__drop-area"), a && b ? b.y + b.height - (a.y + a.height) : 0);
  }, [contentRef, rowsRef]);

  // Every commit: find the row elements, and draw the current frame (or clear) before paint.
  useLayoutEffect(() => {
    latest.current = rows;
    const map = new Map<string, HTMLElement>();
    const content = contentRef.current;
    if (content) {
      for (const el of content.querySelectorAll<HTMLElement>(":scope > .eth-arr-row[data-track]")) map.set(el.dataset.track!, el);
    }
    els.current = map;
    apply();
  });
  // After the arrangement's own `rowsRef = rows` effect: keep the on-screen rows there.
  useEffect(() => {
    rowsRef.current = animatedRows(rows, slotNow);
  });
  useEffect(() => subscribeLaneFrames(apply), [apply]);
}
