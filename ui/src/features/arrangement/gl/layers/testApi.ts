/**
 * The lane layers' part of the arranger test API (`window.__ether.arrangement`, see
 * `../../testApi.ts`): the arranger canvas has no element per automation point, comp piece
 * or take region, so e2e specs aim through these. Without a mounted renderer: null / 0.
 */

import type { AutomationPointId, CompRegionId, TrackId } from "@/generated";
import { valueToY } from "@/features/automation/geometry";
import { compOf } from "@/domain/comping";
import { TAKE_LANE_HEIGHT } from "@/features/comping/height";
import { useCompingUi } from "@/features/comping/store";
import { useProjectStore } from "@/state";
import { CLIP_MIN_PX } from "../../laneBodyGeometry";
import { mainHeight, type Row } from "../../layout";
import { arrangementView, useArrangementUi } from "../../uiStore";
import { glRenderer } from "../active";
import { automationLanesOf } from "./automationLanes";
import { compItems } from "./comping";
import { summarySpans } from "./groupSummary";

export interface ClientRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface LaneTestApi {
  /** Center of an automation point in client px, or null when it isn't shown. */
  automationPoint(id: AutomationPointId): { x: number; y: number } | null;
  /** A comp region's (first) piece on its track's main lane, in client px. */
  compPiece(region: CompRegionId): ClientRect | null;
  /** A comp region's mark in its take lane (shown take lanes only), in client px. */
  takeRegion(region: CompRegionId): ClientRect | null;
  /** The swipe being dragged in a take lane, in client px. */
  swipePreview(): ClientRect | null;
  /** Bars of a group's summary in the arranger's view. */
  groupSummaryBars(track: TrackId): number;
}

interface Plumbing {
  content: () => HTMLElement | null;
  rows: () => ReadonlyArray<Row>;
}

export function laneTestApi(p: Plumbing): LaneTestApi {
  /** A beat span in a row's band (`top`/`height` from the row top) as a client rect. */
  const span = (track: TrackId, b0: number, b1: number, top: number, height: number, minW = 0): ClientRect | null => {
    const content = p.content();
    const row = p.rows().find((r) => r.track.id === track);
    if (!content || !row) return null;
    const v = arrangementView.getState();
    const box = content.getBoundingClientRect();
    const x0 = (b0 - v.scrollBeats) * v.pxPerBeat;
    const x1 = Math.max(x0 + minW, (b1 - v.scrollBeats) * v.pxPerBeat);
    return { x: box.left + useArrangementUi.getState().headerWidth + x0, y: box.top + row.y + top, width: x1 - x0, height };
  };
  return {
    automationPoint: (id) => {
      if (!glRenderer()) return null;
      const project = useProjectStore.getState().project;
      const track = project?.automation_lanes[project.automation_points[id]?.lane ?? ""]?.owner;
      if (!track || track.type !== "Track") return null;
      for (const lane of automationLanesOf(track.track)) {
        const pt = lane.points.find((x) => x.id === id);
        if (!pt) continue;
        const box = lane.el.getBoundingClientRect();
        const v = arrangementView.getState();
        return { x: box.left + (pt.time + lane.offset - v.scrollBeats) * v.pxPerBeat, y: box.top + valueToY(pt.value, lane.height, lane.range) };
      }
      return null;
    },
    compPiece: (region) => {
      if (!glRenderer()) return null;
      const project = useProjectStore.getState().project;
      const r = project?.comp_regions[region];
      const track = r ? project.tracks[r.track] : undefined;
      const row = track && p.rows().find((x) => x.track.id === track.id);
      if (!project || !track || !row) return null;
      const piece = compItems(project, track, null).find((it) => it.region === region);
      return piece ? span(track.id, piece.bounds.start, piece.bounds.start + piece.bounds.length, 0, mainHeight(row), CLIP_MIN_PX) : null;
    },
    takeRegion: (region) => {
      if (!glRenderer()) return null;
      const project = useProjectStore.getState().project;
      const r = project?.comp_regions[region];
      const row = r && p.rows().find((x) => x.track.id === r.track);
      if (!project || !r || !row || !(row.takesHeight ?? 0) || !useCompingUi.getState().expanded.has(r.track)) return null;
      const lanes = Object.values(project.take_lanes)
        .filter((l) => l.track === r.track)
        .sort((a, b) => (a.order < b.order ? -1 : a.order > b.order ? 1 : 0));
      const i = lanes.findIndex((l) => l.id === r.lane);
      if (i < 0 || !compOf(project, r.track).some((x) => x.id === region)) return null;
      return span(r.track, r.start, r.end, mainHeight(row) + i * TAKE_LANE_HEIGHT + 2, TAKE_LANE_HEIGHT - 4);
    },
    swipePreview: () => {
      if (!glRenderer()) return null;
      const s = useCompingUi.getState().swipe;
      const project = useProjectStore.getState().project;
      const row = s && p.rows().find((x) => x.track.id === s.track);
      if (!s || !project || !row) return null;
      const lanes = Object.values(project.take_lanes)
        .filter((l) => l.track === s.track)
        .sort((a, b) => (a.order < b.order ? -1 : a.order > b.order ? 1 : 0));
      const i = lanes.findIndex((l) => l.id === s.lane);
      return i < 0 ? null : span(s.track, s.start, s.end, mainHeight(row) + i * TAKE_LANE_HEIGHT, TAKE_LANE_HEIGHT);
    },
    groupSummaryBars: (track) => {
      if (!glRenderer()) return 0;
      const project = useProjectStore.getState().project;
      const content = p.content();
      if (!project || !content) return 0;
      const v = arrangementView.getState();
      const to = v.scrollBeats + (content.clientWidth - useArrangementUi.getState().headerWidth) / v.pxPerBeat;
      return summarySpans(project, track).filter(([s, l]) => s + l >= v.scrollBeats && s <= to).length;
    },
  };
}
