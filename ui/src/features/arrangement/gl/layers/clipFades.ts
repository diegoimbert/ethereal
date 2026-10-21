/**
 * Clip fades and crossfades on the WebGL arranger: what `ClipFades` (features/clip-editing)
 * drew over audio clips in the DOM, and its gestures.
 *
 * - The fade shapes: the shade above each fade curve and the curve (`fadeGain`), drawn by the
 *   `fade` pipeline; on a crossfaded edge, the tinted overlap and the neighbour's curve
 *   dashed across it (an X with this clip's).
 * - The handles, shown while the clip is hovered: a length handle per fade on
 *   the body's top edge (drag horizontally; past the clip's edge into a touching clip it
 *   creates a crossfade, on a crossfade it resizes it around its centre) and a curve handle
 *   at the fade's midpoint (drag vertically; double-click: linear).
 *
 * Each drag is one undo step (`LaneGesture`), with the same commands and math as `ClipFades`
 * (`dragFadeLength`, `crossfadeDragLength`, `setCrossfadeCommand`, `dragCurve`).
 */

import type { Clip, ClipId, Command, FadeCurve } from "@/generated";
import { openContextMenu, setDragCursor } from "@/kit";
import { useProjectStore } from "@/state";
import { cmd, type EngineTransport } from "@/transport";
import { LaneGesture } from "@/features/automation/gesture";
import {
  audioOf,
  crossfadeDragLength,
  crossfadeSide,
  dragCurve,
  dragFadeLength,
  sendEdit,
  setCrossfadeCommand,
  type CrossfadeSide,
} from "@/features/clip-editing/clipEditing";
import { fullClipMenu } from "../../clipContextMenu";
import { bodyHeight, CLIP_MIN_PX } from "../../laneBodyGeometry";
import type { LaneItem } from "../../laneItems";
import { mainHeight, type Row } from "../../layout";
import { arrangementView } from "../../uiStore";
import type { Hit } from "../hitTest";
import { isSmall } from "../hitTest";
import type { ArrangerLayer, LaneLayerContext, LayerOutput } from "../layers";
import { Fx, type FxCode } from "../pipelines";
import type { ArrangerRenderer } from "../renderer";
import type { LaneScene, SceneInput } from "../scene";
import { editColors } from "./colors";
import type { Pipeline } from "../glContext";
import { FadeMode, writeFade, type FadeSpec } from "./fadePipeline";
import { drawOrder, fadeGeometry, HANDLE_INSET_PX, HANDLE_PX, hitFadeHandle, laneCrossfades, visibleSpans, type FadeHandle } from "./fadeModel";

export const CLIP_FADES_LAYER = "clip-fades";

/** Curve of a new crossfade made by dragging a fade handle into the neighbour (as `ClipFades`). */
const NEW_CROSSFADE_CURVE: FadeCurve = { type: "EqualPower" };
/** Beats standing for "no clip above covers this side" (instances are in beats). */
const FAR = 1e7;

type Side = "in" | "out";

/** What the layer drew for a clip (test API). */
export interface FadeDecor {
  /** Fade curves drawn (`.eth-clip-fades__line`, neighbours' included). */
  lines: number;
  crossfadeIn: boolean;
  crossfadeOut: boolean;
  /** Handles shown (the clip is hovered). */
  handles: boolean;
}

export interface ClipFadesLayer extends ArrangerLayer {
  /** The overlay last built for a clip (null: not built, e.g. off screen or not audio). */
  decor(id: ClipId): FadeDecor | null;
}

/** The hovered clip and part key of a lane (`input.hover` within it), "" when none. */
function laneHover(lane: LaneScene, input: SceneInput, idsOf: (lane: LaneScene) => ReadonlySet<ClipId>): string {
  const h = input.hover;
  if (!h || !idsOf(lane).has(h.clip)) return "";
  return `${h.clip}|${h.part}|${typeof h.data === "string" ? h.data : ""}`;
}

export function clipFadesLayer(renderer: ArrangerRenderer, transport: EngineTransport, pipeline: Pipeline): ClipFadesLayer {
  const decors = new Map<ClipId, FadeDecor>();
  const laneIds = new WeakMap<ReadonlyArray<LaneItem>, Set<ClipId>>();
  const idsOf = (lane: LaneScene) => {
    let ids = laneIds.get(lane.items);
    if (!ids) laneIds.set(lane.items, (ids = new Set(lane.items.map((it) => it.clip.id))));
    return ids;
  };

  const build = (ctx: LaneLayerContext, out: LayerOutput) => {
    const { lane, input } = ctx;
    const w = out.custom(pipeline);
    const colors = editColors(input.palette);
    const order = drawOrder(lane.items, input.selected);
    const spans = visibleSpans(order);
    const xfades = laneCrossfades(lane.items);
    const frozen = lane.frozen;
    const hover = input.hover;
    const view = { scrollBeats: 0, pxPerBeat: ctx.window.ppb };
    order.forEach((it, k) => {
      const a = audioOf(it.clip);
      if (!a || it.ghost || isSmall(it, view)) {
        decors.delete(it.clip.id);
        return;
      }
      const { clip, bounds } = it;
      const len = bounds.length;
      const s = bounds.start;
      const e = s + len;
      const box = { b0: s, b1: e, minW: CLIP_MIN_PX };
      const fx: FxCode = frozen ? Fx.Frozen : clip.muted ? Fx.Muted : Fx.None;
      const alpha = it.dragging ? 0.85 : 1;
      const fin = Math.min(a.fade_in, len);
      const fout = Math.min(a.fade_out, len);
      const xv = xfades.get(clip.id) ?? { in: null, out: null };
      const hovered = hover?.clip === clip.id;
      // Only on the hovered clip (a selected one shows its fades, not their handles).
      const handles = hovered;
      const hoveredHandle = hovered && typeof hover?.data === "string" ? hover.data : null;
      const decor: FadeDecor = { lines: 0, crossfadeIn: !!xv.in && fin > 0, crossfadeOut: !!xv.out && fout > 0, handles };
      const zoomed = (beats: number) => beats * ctx.window.ppb;
      decor.lines += (fin > 0 && zoomed(fin) > 0.5 ? 1 : 0) + (fout > 0 && zoomed(fout) > 0.5 ? 1 : 0);
      decor.lines += (xv.in && zoomed(fin) > 0.5 ? 1 : 0) + (xv.out && zoomed(fout) > 0.5 ? 1 : 0);
      decors.set(clip.id, decor);
      for (const [v0, v1] of spans[k]!) {
        const vis: [number, number] = [v0 === -Infinity ? -FAR : v0, v1 === Infinity ? FAR : v1];
        const put = (spec: Omit<FadeSpec, "box" | "vis" | "fx" | "alpha">) => writeFade(w, { ...spec, box, vis, fx, alpha });
        // Crossfade regions under the curves.
        if (xv.in && fin > 0) put({ mode: FadeMode.Tint, f0: s, f1: s + fin, fill: colors.tint });
        if (xv.out && fout > 0) put({ mode: FadeMode.Tint, f0: e - fout, f1: e, fill: colors.tint });
        for (const side of ["in", "out"] as const) {
          const f = side === "in" ? fin : fout;
          if (!(f > 0)) continue;
          const [f0, f1] = side === "in" ? [s, s + f] : [e - f, e];
          const curve = side === "in" ? a.fade_in_curve : a.fade_out_curve;
          const dir = side === "in" ? 1 : -1;
          const neighbour = xv[side];
          if (!neighbour) put({ mode: FadeMode.Shade, f0, f1, curve, dir, fill: colors.shade });
          // The neighbour's fade across the crossfade (its fade-out over our fade-in, and back).
          else put({ mode: FadeMode.Dashed, f0, f1, curve: neighbour, dir: dir === 1 ? -1 : 1, fill: colors.neighbour });
          put({ mode: FadeMode.Line, f0, f1, curve, dir, fill: colors.line });
        }
        if (!handles) continue;
        const fill = (h: FadeHandle) => (hoveredHandle === h ? colors.handleHover : colors.line);
        const handle = { stroke: colors.handleBorder };
        put({ ...handle, mode: FadeMode.Handle, f0: s + fin, f1: s + fin, fill: fill("fade-in"), minFromStart: HANDLE_INSET_PX, radius: 2.5 });
        put({ ...handle, mode: FadeMode.Handle, f0: e - fout, f1: e - fout, fill: fill("fade-out"), minFromEnd: HANDLE_INSET_PX, radius: 2.5 });
        put({ ...handle, mode: FadeMode.CurveHandle, f0: s, f1: s + fin, curve: a.fade_in_curve, fill: fill("fade-in-curve"), radius: HANDLE_PX / 2 });
        put({ ...handle, mode: FadeMode.CurveHandle, f0: e - fout, f1: e, curve: a.fade_out_curve, dir: -1, fill: fill("fade-out-curve"), radius: HANDLE_PX / 2 });
      }
    });
  };

  /** The handle under lane px `x`, row px `y` (with its clip), or null. */
  const handleAt = (x: number, y: number, row: Row): { it: LaneItem; handle: FadeHandle } | null => {
    const lane = renderer.laneOf(row.track.id);
    const input = renderer.getInput();
    if (!lane || !input) return null;
    const v = arrangementView.getState();
    const h = mainHeight(row);
    // The topmost clip under the pointer.
    const order = drawOrder(lane.items, input.selected);
    for (let i = order.length - 1; i >= 0; i--) {
      const it = order[i]!;
      if (it.ghost || isSmall(it, v)) continue;
      const g = fadeGeometry(it, v, h);
      const [x0, x1] = g ? [g.x0, g.x1] : [Infinity, -Infinity];
      if (!g) {
        // Not audio: does it cover the point? Then nothing below gets it.
        const x0b = (it.bounds.start - v.scrollBeats) * v.pxPerBeat;
        if (x >= x0b && x < Math.max(x0b + CLIP_MIN_PX, x0b + it.bounds.length * v.pxPerBeat)) return null;
        continue;
      }
      if (x < x0 || x >= x1) continue;
      const handle = hitFadeHandle(g, x, y);
      return handle ? { it, handle } : null;
    }
    return null;
  };

  /** The crossfade side under a handle, from the current document. */
  const sideNow = (clip: Clip, side: Side): CrossfadeSide | null => {
    const p = useProjectStore.getState().project;
    const c = p?.clips[clip.id];
    return p && c ? crossfadeSide(p, c, side) : null;
  };

  /** A curve change: both clips' fades on a crossfaded edge, else this clip's fade. */
  const curveCommand = (clip: Clip, side: Side, curve: FadeCurve): Command => {
    const xf = sideNow(clip, side);
    if (xf?.crossfaded) return setCrossfadeCommand(xf, xf.length, curve);
    return cmd("Clip", { type: "SetFadeCurves", id: clip.id, fade_in: side === "in" ? curve : null, fade_out: side === "out" ? curve : null });
  };

  const track = (onMove: (ev: PointerEvent) => void, cursor: string, gesture: LaneGesture) => {
    setDragCursor(cursor);
    const up = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
      setDragCursor(null);
      gesture.end();
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
  };

  /** Fade length drag (as `ClipFades`' `startLengthDrag`): fades, then crossfades past the edge. */
  const lengthDrag = (clip: Clip, side: Side, e: PointerEvent) => {
    const a = audioOf(clip);
    if (!a) return;
    const gesture = new LaneGesture(transport);
    const x0 = e.clientX;
    const [start, other] = side === "in" ? [a.fade_in, a.fade_out] : [a.fade_out, a.fade_in];
    const xf = sideNow(clip, side);
    // The beat under the pointer when the drag starts.
    const v = arrangementView.getState();
    // The canvas starts `bleed` px before the lanes (it draws under the header column).
    const laneLeft = renderer.canvas.getBoundingClientRect().left + (renderer.getViewport().bleed ?? 0);
    const handleBeat = v.scrollBeats + (e.clientX - laneLeft) / Math.max(1e-9, v.pxPerBeat);
    let creating = false;
    track(
      (ev) => {
        const dx = ev.clientX - x0;
        const ppb = arrangementView.getState().pxPerBeat;
        const pointer = handleBeat + dx / Math.max(1e-9, ppb);
        let command: Command | null = null;
        if (xf?.crossfaded) {
          command = setCrossfadeCommand(xf, crossfadeDragLength(xf.centre, pointer, side === "out", ppb), null);
        } else if (xf && (creating || (side === "out" ? pointer > xf.centre : pointer < xf.centre))) {
          const len = crossfadeDragLength(xf.centre, pointer, side === "in", ppb);
          if (len > 0 || creating) {
            creating = true;
            command = setCrossfadeCommand(xf, len, NEW_CROSSFADE_CURVE);
          }
        }
        if (!command) {
          const fade = dragFadeLength(side, start, dx, ppb, clip.length, other);
          const [fade_in, fade_out] = side === "in" ? [fade, a.fade_out] : [a.fade_in, fade];
          command = cmd("Clip", { type: "SetFades", id: clip.id, fade_in, fade_out });
        }
        gesture.update(command);
      },
      "ew-resize",
      gesture,
    );
  };

  /** Curve drag (as `ClipFades`' `startCurveDrag`): vertical moves bend the curve. */
  const curveDrag = (clip: Clip, side: Side, e: PointerEvent, bodyH: number) => {
    const a = audioOf(clip);
    if (!a) return;
    const gesture = new LaneGesture(transport);
    const y0 = e.clientY;
    const start = side === "in" ? a.fade_in_curve : a.fade_out_curve;
    track((ev) => gesture.update(curveCommand(clip, side, dragCurve(start, (ev.clientY - y0) / Math.max(1, bodyH)))), "ns-resize", gesture);
  };

  /** The row of the last hit (a press follows its hit test). */
  let lastRow: Row | null = null;

  const hitData = (hit: Hit): { clip: Clip; handle: FadeHandle; side: Side } | null => {
    const clip = useProjectStore.getState().project?.clips[hit.clip];
    const handle = hit.data as FadeHandle;
    if (!clip || typeof handle !== "string") return null;
    return { clip, handle, side: handle.startsWith("fade-in") ? "in" : "out" };
  };

  return {
    id: CLIP_FADES_LAYER,
    z: "over-clips",
    pipelines: [pipeline],
    deps: (ctx) => [ctx.lane.staticVersion, ctx.window.ppb, laneHover(ctx.lane, ctx.input, idsOf)],
    build,
    hitTest(x, y, row) {
      const found = handleAt(x, y, row);
      if (found) lastRow = row;
      return found ? { clip: found.it.clip.id, track: row.track.id, part: "layer", data: found.handle } : null;
    },
    onPointerDown(hit, e) {
      if (e.button !== 0) return;
      const d = hitData(hit);
      if (!d) return;
      e.stopPropagation();
      e.preventDefault();
      if (d.handle === "fade-in" || d.handle === "fade-out") {
        lengthDrag(d.clip, d.side, e);
      } else {
        curveDrag(d.clip, d.side, e, lastRow?.track.id === hit.track ? bodyHeight(mainHeight(lastRow)) : 1);
      }
    },
    onDoubleClick(hit, e) {
      const d = hitData(hit);
      e.stopPropagation();
      if (!d || !d.handle.endsWith("-curve")) return;
      void sendEdit(transport, curveCommand(d.clip, d.side, { type: "Linear" }));
    },
    onContextMenu(hit, e) {
      const clip = useProjectStore.getState().project?.clips[hit.clip];
      if (clip) openContextMenu(e, fullClipMenu(transport, clip));
    },
    cursor: (hit) => (typeof hit.data === "string" && hit.data.endsWith("-curve") ? "ns-resize" : "ew-resize"),
    decor: (id) => decors.get(id) ?? null,
  };
}
