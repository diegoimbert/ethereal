/**
 * New-track ghost lanes on the WebGL arranger: while clips are dragged below the last track
 * (`newTrackDrag.ts`), the tracks the drop would create are shown as ghost rows
 * (`NewTrackGhostRows`, DOM: header, dashed lane outline) and their clips are drawn here,
 * as lanes of stand-in tracks under the last row (the DOM rows hold
 * the ghost rows). The scene already places the dragged clips in these lanes: the drag
 * preview moves them to the new lanes' ids.
 */

import type { Track } from "@/generated";
import { rowsHeight, type Row } from "../../layout";
import type { NewLane } from "../../newTrackDrag";
import { useArrangementUi } from "../../uiStore";
import type { ArrangerRenderer } from "../renderer";

/** The ghost rows of `lanes`, stacked under `rows` (each `height` tall), as `NewTrackGhostRows` lays them out. */
export function ghostLaneRows(lanes: ReadonlyArray<NewLane>, rows: ReadonlyArray<Row>, height: number): Row[] {
  const top = rowsHeight(rows);
  return lanes.map((lane, i) => ({
    track: standInTrack(lane),
    depth: 0,
    y: top + i * height,
    laneHeight: height,
    height,
  }));
}

/** A stand-in track for a lane the drop would create (only what the lane scene reads). */
function standInTrack(lane: NewLane): Track {
  return {
    id: lane.id,
    kind: lane.kind,
    name: "",
    color: lane.color,
    order: "",
    parent: null,
    freeze: null,
    mixer: { volume: 0, pan: 0, mute: false, solo: false },
  } as unknown as Track;
}

/** Draws the ghost lanes on `renderer` (rows: the arrangement's rows); returns the removal. */
export function mountGhostLanes(renderer: ArrangerRenderer, rows: () => ReadonlyArray<Row>): () => void {
  let cache: { lanes: ReadonlyArray<NewLane>; top: number; height: number; rows: Row[] } | null = null;
  const extra = (): ReadonlyArray<Row> => {
    const ui = useArrangementUi.getState();
    const lanes = ui.preview?.newTracks;
    if (!lanes?.length) {
      cache = null;
      return [];
    }
    const r = rows();
    const top = rowsHeight(r);
    // Same stand-in tracks for the whole drag (the lane scenes keep their instances).
    if (!cache || cache.lanes !== lanes || cache.top !== top || cache.height !== ui.defaultHeight) {
      cache = { lanes, top, height: ui.defaultHeight, rows: ghostLaneRows(lanes, r, ui.defaultHeight) };
    }
    return cache.rows;
  };
  return renderer.setExtraRows(extra);
}
