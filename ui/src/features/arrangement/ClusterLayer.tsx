import { useLayoutEffect, useRef } from "react";
import type { Beats, ClipId, Color } from "@/generated";
import type { ClipCluster } from "./clipClusters";
import { colorCss } from "./helpers";
import { beatsCss } from "./laneGeometry";

/** Longest canvas side in device px. */
const MAX_CANVAS_PX = 8192;
/** Vertical inset of the bars, like clips (px). */
const INSET = 2;
/** Height of the bright band on top of a bar, like a clip's title (px). */
const BAND = 3;

export interface ClusterLayerProps {
  clusters: ReadonlyArray<ClipCluster>;
  /** Layout origin of the lane layer (beats) and the render window. */
  origin: Beats;
  visible: { start: Beats; end: Beats };
  /** Settled zoom the canvas is drawn at (it stretches with --ppb until the next render). */
  pxPerBeat: number;
  trackColor: Color;
  selected: ReadonlySet<ClipId>;
}

/**
 * The lane's clip clusters (see clipClusters.ts), drawn as bars on a single canvas: one per
 * lane, whatever the number of clips. Each clip is a segment in its own color (gaps show
 * through), with a bright band on top; a selected clip lights up its segment and outlines
 * its cluster. Positioned in beats (`--ppb`), so zoom and scroll only stretch and move it.
 */
export function ClusterLayer({ clusters, origin, visible, pxPerBeat, trackColor, selected }: ClusterLayerProps) {
  const ref = useRef<HTMLCanvasElement>(null);
  const from = visible.start;
  const to = visible.end;

  useLayoutEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const dpr = window.devicePixelRatio || 1;
    const w = Math.max(1, Math.round((to - from) * pxPerBeat));
    const h = canvas.clientHeight || 40;
    canvas.width = Math.min(MAX_CANVAS_PX, Math.round(w * dpr));
    canvas.height = Math.round(h * dpr);
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(canvas.width / w, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
    const x = (b: Beats) => (b - from) * pxPerBeat;
    const top = INSET;
    const bottom = h - INSET;
    for (const c of clusters) {
      if (c.end < from || c.start > to) continue;
      const ghost = c.items[0]!.ghost;
      let anySelected = false;
      for (const it of c.items) {
        const color = colorCss(it.clip.color ?? trackColor);
        const sel = !ghost && selected.has(it.clip.id);
        anySelected ||= sel;
        const x0 = x(it.bounds.start);
        const x1 = Math.max(x0 + 1, x(it.bounds.start + it.bounds.length));
        ctx.globalAlpha = (ghost ? 0.45 : it.dragging ? 0.8 : 1) * (it.clip.muted ? 0.4 : 1);
        ctx.fillStyle = color;
        ctx.globalAlpha *= sel ? 0.7 : 0.32;
        ctx.fillRect(x0, top, x1 - x0, bottom - top);
        ctx.globalAlpha /= sel ? 0.7 : 0.32;
        ctx.fillRect(x0, top, x1 - x0, BAND);
        // Clip boundaries: a dark hairline where neighbours touch.
        if (x1 - x0 >= 3) {
          ctx.globalAlpha = 0.35;
          ctx.fillStyle = "#000";
          ctx.fillRect(Math.round(x1) - 1, top, 1, bottom - top);
        }
      }
      ctx.globalAlpha = ghost ? 0.45 : 1;
      ctx.strokeStyle = colorCss(c.items[0]!.clip.color ?? trackColor);
      ctx.lineWidth = anySelected ? 2 : 1;
      const x0 = x(c.start);
      const x1 = Math.max(x0 + 2, x(c.end));
      ctx.beginPath();
      ctx.roundRect(x0 + 0.5, top + 0.5, x1 - x0 - 1, bottom - top - 1, 3);
      ctx.stroke();
    }
    ctx.globalAlpha = 1;
  });

  return (
    <canvas
      ref={ref}
      className="eth-arr-clusters"
      data-testid="clip-clusters"
      data-clusters={clusters.length}
      style={{ left: beatsCss(from - origin), width: beatsCss(to - from) }}
    />
  );
}
