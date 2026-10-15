/**
 * Comping on the WebGL arranger (the comp's pieces and the take lanes' clips):
 *
 * - **the comp** on the main lane: each piece is its take clip trimmed to the region, drawn
 *   like a clip (an extra lane over the main lane) with a dashed outline (a solid focus ring
 *   for the selected region), dimmed while a take is auditioned. A click on a piece selects
 *   its region and opens the take lanes (the layer's hit test and gesture).
 * - **take lanes** under the main lane: the take's clips at `--eth-opacity-muted` (full while
 *   auditioned), the regions this take plays (a lit window in the take's color), and the
 *   swipe being dragged. The lanes' DOM keeps the headers and the swipe gesture.
 */

import type { Clip, CompRegion, Project, TakeLane, Track, TrackId } from "@/generated";
import { compOf, compPieces, laneClipsOf, lanesOf } from "@/domain/comping";
import { TAKE_LANE_HEIGHT } from "@/features/comping/height";
import { useCompingUi } from "@/features/comping/store";
import type { TempoMap } from "@/timeline";
import { CLIP_INSET, CLIP_MIN_PX, CLIP_RADIUS } from "../../laneBodyGeometry";
import { stableBoundsOf, type LaneItem } from "../../laneItems";
import { mainHeight } from "../../layout";
import { arrangementView } from "../../uiStore";
import { glRenderer } from "../active";
import type { Pipeline } from "../glContext";
import type { ArrangerLayer, ExtraLane, LaneLayerContext } from "../layers";
import { rgbOf, withAlpha } from "../palette";
import { writeRect } from "../pipelines";
import type { LaneSource, SceneInput } from "../scene";
import { lanePalette } from "./lanePalette";
import { writeOutline } from "./shapes";

/** `--eth-opacity-muted`: take clips, and the comp while a take is auditioned. */
export const MUTED_OPACITY = 0.6;
/** Comp regions in a take lane (`.eth-takes__region`): inset, radius, ring. */
const REGION_INSET = 2;
const REGION_RADIUS = 6;
const RING = 2;
/** The comp's piece outline (`outline: 1px dashed`, offset -1px): dash and gap (px). */
const DASH = 3;

const keyOf = (...parts: string[]) => parts.join(":");

/** One comp piece as drawn: its take clip reshaped to the piece, with the take's name. */
export interface CompItem extends LaneItem {
  region: string;
}

/** The comp pieces of a track as lane items (memoized on the tables they come from). */
export function compItems(project: Pick<Project, "clips" | "comp_regions" | "tracks" | "take_lanes">, track: Track, tempo: TempoMap | null): CompItem[] {
  const p = { clips: project.clips, comp_regions: project.comp_regions, tracks: { [track.id]: track } };
  const out: CompItem[] = [];
  for (const piece of compPieces(p, track.id, tempo ? (b) => tempo.bpmAt(b) : undefined)) {
    const clip = project.clips[piece.clip];
    if (!clip) continue;
    const shaped: Clip = {
      ...clip,
      name: project.take_lanes[piece.lane]?.name ?? clip.name,
      start: piece.start,
      length: piece.end - piece.start,
      offset: piece.offset,
    };
    out.push({
      clip: shaped,
      bounds: { track: track.id, start: piece.start, length: piece.end - piece.start, offset: piece.offset },
      dragging: false,
      ghost: false,
      region: piece.region,
    });
  }
  return out;
}

/** Lane sources kept per key (the renderer keeps an extra lane's scene while its source is the same). */
class Sources {
  private map = new Map<string, LaneSource>();
  get(key: string, make: () => LaneSource): LaneSource {
    let s = this.map.get(key);
    if (!s) this.map.set(key, (s = make()));
    return s;
  }
}

/** A memo on reference-equal inputs. */
function memo<A extends readonly unknown[], R>(fn: (...args: A) => R): (...args: A) => R {
  let last: { args: A; out: R } | null = null;
  return (...args: A) => {
    if (last && last.args.length === args.length && last.args.every((a, i) => a === args[i])) return last.out;
    const out = fn(...args);
    last = { args, out };
    return out;
  };
}

function compSource(track: TrackId): LaneSource & { pieces(input: SceneInput): CompItem[] } {
  const pieces = memo((clips: Project["clips"], regions: Project["comp_regions"], lanes: Project["take_lanes"], t: Track | undefined, tempo: TempoMap) =>
    t ? compItems({ clips, comp_regions: regions, take_lanes: lanes, tracks: {} }, t, tempo) : [],
  );
  const get = (input: SceneInput) =>
    pieces(input.project.clips, input.project.comp_regions, input.project.take_lanes, input.project.tracks[track], input.tempo);
  return { items: get, pieces: get, elements: true };
}

function takeSource(lane: string): LaneSource {
  const items = memo((clips: Project["clips"]) =>
    laneClipsOf({ clips }, lane).map((clip): LaneItem => ({ clip, bounds: stableBoundsOf(clip), dragging: false, ghost: false })),
  );
  return { items: (input) => items(input.project.clips), elements: true };
}

/** The take lanes shown under a track (none when collapsed). */
function shownTakes(project: Project, track: TrackId): TakeLane[] {
  return useCompingUi.getState().expanded.has(track) ? lanesOfMemo(project.take_lanes, track) : [];
}
const takesCache = new Map<TrackId, { lanes: Project["take_lanes"]; out: TakeLane[] }>();
function lanesOfMemo(lanes: Project["take_lanes"], track: TrackId): TakeLane[] {
  const c = takesCache.get(track);
  if (c && c.lanes === lanes) return c.out;
  const out = lanesOf({ take_lanes: lanes }, track);
  takesCache.set(track, { lanes, out });
  return out;
}

const isClipTrack = (t: Track) => t.kind === "Audio" || t.kind === "Midi";

/**
 * The comping layers: `under` draws the take lanes' background (under their clips), `over`
 * the regions, the swipe and the comp's outlines, and adds the extra lanes (the comp, the
 * take lanes) and the comp's hit test.
 */
export function compingLayers(shape: Pipeline): { under: ArrangerLayer; over: ArrangerLayer } {
  const comps = new Map<TrackId, ReturnType<typeof compSource>>();
  const takes = new Sources();
  const comp = (track: TrackId) => {
    let s = comps.get(track);
    if (!s) comps.set(track, (s = compSource(track)));
    return s;
  };

  const under: ArrangerLayer = {
    id: "comping-takes",
    z: "under-clips",
    deps: ({ row, input }) => [row.takesHeight ?? 0, row.laneHeight, input.palette],
    build({ row, input }, out) {
      const h = row.takesHeight ?? 0;
      if (h <= 0) return;
      const main = mainHeight(row);
      // `.eth-takes`: color-mix(bg-inset 70%, transparent).
      writeRect(out.rects, { x: { b0: -1e6, b1: 1e6 }, y: { a0: main, a1: main + h }, fill: withAlpha(lanePalette(input.palette).bgInset, 0.7) });
    },
  };

  const over: ArrangerLayer = {
    id: "comping",
    pipelines: [shape],
    z: "over-clips",
    lanes({ track, row, input }: LaneLayerContext): ExtraLane[] {
      if (!isClipTrack(track)) return [];
      const ui = useCompingUi.getState();
      const out: ExtraLane[] = [];
      const auditioned = ui.audition.get(track.id) ?? null;
      const c = comp(track.id);
      if (c.pieces(input).length > 0)
        out.push({ key: keyOf("comp", track.id), top: 0, height: mainHeight(row), source: c, alpha: auditioned ? MUTED_OPACITY : 1 });
      if ((row.takesHeight ?? 0) > 0) {
        shownTakes(input.project, track.id).forEach((lane, i) => {
          out.push({
            key: keyOf("take", lane.id),
            top: mainHeight(row) + i * TAKE_LANE_HEIGHT,
            height: TAKE_LANE_HEIGHT,
            source: takes.get(lane.id, () => takeSource(lane.id)),
            alpha: auditioned === lane.id ? 1 : MUTED_OPACITY,
          });
        });
      }
      return out;
    },
    deps: ({ track, row, input }) => {
      const ui = useCompingUi.getState();
      return [
        isClipTrack(track) ? comp(track.id).pieces(input) : null,
        input.project.comp_regions,
        input.project.take_lanes,
        track.color,
        ui.selected,
        ui.swipe,
        ui.audition,
        ui.expanded,
        row.laneHeight,
        row.takesHeight ?? 0,
        input.palette,
      ];
    },
    build({ track, row, input }, out) {
      if (!isClipTrack(track)) return;
      const ui = useCompingUi.getState();
      const c = lanePalette(input.palette);
      const main = mainHeight(row);
      const selected = ui.selected?.track === track.id ? ui.selected.region : null;
      const auditioned = ui.audition.get(track.id) ?? null;
      const w = out.custom(shape);
      // The comp's outlines (`.eth-comp__piece .eth-clip--ghost`).
      const dashed = withAlpha(c.text, 0.45);
      for (const it of comp(track.id).pieces(input)) {
        const b0 = it.bounds.start;
        const b1 = it.bounds.start + it.bounds.length;
        const clip = { y0: 0, y1: main };
        const alpha = auditioned ? MUTED_OPACITY : 1;
        if (it.region === selected) {
          writeOutline(
            w,
            { beat: b0, px: -1, y: CLIP_INSET - 1 },
            { beat: b1, px: 1, y: main - CLIP_INSET + 1 },
            { radius: CLIP_RADIUS + 1, width: RING, color: c.focus, minW: CLIP_MIN_PX + 2, alpha, clip },
          );
        } else {
          writeOutline(
            w,
            { beat: b0, y: CLIP_INSET },
            { beat: b1, y: main - CLIP_INSET },
            { radius: CLIP_RADIUS, width: 1, color: dashed, dash: DASH, minW: CLIP_MIN_PX, alpha, clip },
          );
        }
      }
      // Take lanes: regions and the swipe.
      if ((row.takesHeight ?? 0) <= 0) return;
      const regions = compOf(input.project, track.id);
      shownTakes(input.project, track.id).forEach((lane, i) => {
        const top = main + i * TAKE_LANE_HEIGHT;
        const color = rgbOf(lane.color ?? track.color);
        for (const r of regions) if (r.lane === lane.id) writeRegion(out.rects, r, top, color, r.id === selected ? c.focus : null);
        const s = ui.swipe;
        if (s && s.lane === lane.id) {
          writeRect(out.rects, {
            x: { b0: s.start, b1: s.end },
            y: { a0: top, a1: top + TAKE_LANE_HEIGHT },
            fill: withAlpha(color, 0.35),
            stroke: color,
            strokeWidth: 1,
          });
        }
      });
    },
    hitTest(x, y, row) {
      // A comp piece (the whole lane height, like `.eth-comp__piece`).
      if (y < 0 || y >= mainHeight(row) || !isClipTrack(row.track)) return null;
      const input = glRenderer()?.getInput();
      if (!input) return null;
      const v = arrangementView.getState();
      const pieces = comp(row.track.id).pieces(input);
      for (let i = pieces.length - 1; i >= 0; i--) {
        const p = pieces[i]!;
        const x0 = (p.bounds.start - v.scrollBeats) * v.pxPerBeat;
        const x1 = Math.max(x0 + CLIP_MIN_PX, (p.bounds.start + p.bounds.length - v.scrollBeats) * v.pxPerBeat);
        if (x >= x0 && x < x1) return { clip: p.clip.id, track: row.track.id, part: "layer", data: { region: p.region } };
      }
      return null;
    },
    onPointerDown(hit, e) {
      const region = (hit.data as { region?: string } | undefined)?.region;
      if (!region) return;
      e.stopPropagation();
      useCompingUi.getState().select({ track: hit.track, region });
      useCompingUi.getState().toggle(hit.track, true);
    },
    cursor: () => "pointer",
  };
  return { under, over };
}

function writeRegion(
  rects: Parameters<typeof writeRect>[0],
  r: CompRegion,
  top: number,
  color: readonly [number, number, number, number],
  focus: readonly [number, number, number, number] | null,
): void {
  const x = { b0: r.start, b1: r.end };
  const y = { a0: top + REGION_INSET, a1: top + TAKE_LANE_HEIGHT - REGION_INSET };
  // The selected region's focus ring (`0 0 0 2px focus`, outside the box).
  if (focus)
    writeRect(rects, {
      x: { ...x, p0: -RING, p1: RING },
      y: { a0: y.a0 - RING, a1: y.a1 + RING },
      radius: REGION_RADIUS + RING,
      stroke: focus,
      strokeWidth: RING,
    });
  writeRect(rects, { x, y, radius: REGION_RADIUS, fill: withAlpha(color, 0.22), stroke: color, strokeWidth: RING });
}
