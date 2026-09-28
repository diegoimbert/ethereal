// Arranger geometry shared by presence-v2's `PresenceLayer` (pointers, follow mode) and
// collab-social's overlays (notes, peers' playheads): rows and boxes in root px, measured from
// the DOM and the local layout. Pure DOM reads, no React.
import type { TrackId } from "@/generated";
import { useProjectStore } from "@/state";
import type { Row } from "@/features/arrangement/layout";
import { arrangementView, useArrangementUi } from "@/features/arrangement/uiStore";
import type { Box, FreeSpace, Lanes, RowBox } from "../presence/coords";

const DRAFT = "__draft_track__";

export const parentOf = (id: TrackId): TrackId | null | undefined => {
  const t = useProjectStore.getState().project?.tracks[id];
  return t ? t.parent : undefined;
};

export interface Geometry {
  rows: RowBox[];
  lanes: Lanes;
  /** Visible scrolling lanes and the master lane (root px). */
  scrollBox: Box;
  masterBox: Box | null;
  masterTrack: TrackId | null;
  /** The ruler band (pointers over the ruler sit in its middle). */
  rulerTop: number;
  rulerY: number;
  /** Free space below the last scrolling track, down to the bottom of the view. */
  free: FreeSpace;
}

/** Rows and boxes in root px (from the DOM and the local layout). */
export function measure(root: HTMLElement, scroll: HTMLElement, rows: ReadonlyArray<Row>, masterRow: Row | null): Geometry {
  const r = root.getBoundingClientRect();
  const s = scroll.getBoundingClientRect();
  const hw = useArrangementUi.getState().headerWidth;
  const v = arrangementView.getState();
  const offY = s.top - r.top - scroll.scrollTop;
  const x0 = s.left - r.left + hw;
  const x1 = s.left - r.left + scroll.clientWidth;
  const boxes: RowBox[] = rows.filter((row) => row.track.id !== DRAFT).map((row) => ({ track: row.track.id, top: offY + row.y, height: row.height }));
  const scrollBox = { x0, x1, y0: s.top - r.top, y1: s.top - r.top + scroll.clientHeight };
  const last = rows.at(-1);
  const free = { top: offY + (last ? last.y + last.height : 0), bottom: scrollBox.y1 };
  let masterBox: Box | null = null;
  const masterEl = masterRow ? root.querySelector<HTMLElement>('[data-testid="arrangement-master"]') : null;
  if (masterRow && masterEl) {
    const m = masterEl.getBoundingClientRect();
    masterBox = { x0, x1, y0: m.top - r.top, y1: m.bottom - r.top };
    boxes.push({ track: masterRow.track.id, top: m.top - r.top, height: m.height });
  }
  const ruler = root.querySelector<HTMLElement>(".eth-arr__top")?.getBoundingClientRect();
  const rulerTop = ruler ? ruler.top - r.top : scrollBox.y0;
  return {
    rows: boxes,
    lanes: { left: x0, pxPerBeat: v.pxPerBeat, scrollBeats: v.scrollBeats },
    scrollBox,
    masterBox,
    masterTrack: masterRow?.track.id ?? null,
    rulerTop,
    rulerY: ruler ? (ruler.top + ruler.bottom) / 2 - r.top : scrollBox.y0,
    free,
  };
}
