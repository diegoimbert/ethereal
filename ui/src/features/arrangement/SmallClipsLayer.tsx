import { useLayoutEffect, useRef } from "react";
import type { Beats, ClipId, Color } from "@/generated";
import { colorCss } from "./helpers";
import { beatsCss } from "./laneGeometry";
import type { LaneItem } from "./laneItems";

/** Longest canvas side in device px. */
const MAX_CANVAS_PX = 8192;
/** Vertical inset like clips (px). */
const INSET = 2;
/** Title band height, like `.eth-clip__title` (px). */
const TITLE = 14;
const RADIUS = 4;

export interface SmallClipsLayerProps {
  items: ReadonlyArray<LaneItem>;
  /** Layout origin of the lane layer (beats) and the render window. */
  origin: Beats;
  visible: { start: Beats; end: Beats };
  /** Settled zoom the canvas is drawn at (it stretches with --ppb until the next render). */
  pxPerBeat: number;
  trackColor: Color;
  selected: ReadonlySet<ClipId>;
}

/**
 * Clips too small to be elements (see smallClips.ts), painted on one canvas per lane to
 * look like `ClipView`s: a tinted body, a title band in the clip's color (with the name
 * when it fits), a glow in the clip's color when selected. Positioned in beats (`--ppb`),
 * so zoom and scroll only stretch and move it.
 */
export function SmallClipsLayer({ items, origin, visible, pxPerBeat, trackColor, selected }: SmallClipsLayerProps) {
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
    const top = INSET;
    const height = h - 2 * INSET;
    const title = Math.min(TITLE, height / 3);
    const bodyColor = getComputedStyle(canvas).getPropertyValue("--eth-color-bg-panel").trim() || "#1c1f26";
    const textColor = getComputedStyle(canvas).getPropertyValue("--eth-color-text-inverse").trim() || "#14161b";
    ctx.font = `500 10px ${getComputedStyle(canvas).fontFamily || "sans-serif"}`;
    ctx.textBaseline = "middle";
    for (const it of items) {
      const end = it.bounds.start + it.bounds.length;
      if (end < from || it.bounds.start > to) continue;
      const color = colorCss(it.clip.color ?? trackColor);
      const x0 = (it.bounds.start - from) * pxPerBeat;
      const cw = Math.max(1, it.bounds.length * pxPerBeat);
      const sel = !it.ghost && selected.has(it.clip.id);
      const r = cw >= 2 * RADIUS ? RADIUS : 0;
      ctx.globalAlpha = (it.ghost ? 0.45 : it.dragging ? 0.85 : 1) * (it.clip.muted ? 0.55 : 1);
      if (sel) {
        ctx.shadowColor = color;
        ctx.shadowBlur = 10;
      }
      // Body: the clip color mixed into the panel, like `.eth-clip`.
      ctx.fillStyle = bodyColor;
      roundRect(ctx, x0, top, cw, height, r);
      ctx.fill();
      ctx.shadowBlur = 0;
      ctx.globalAlpha *= 0.26;
      ctx.fillStyle = color;
      ctx.fill();
      ctx.globalAlpha /= 0.26;
      // Title band.
      ctx.save();
      roundRect(ctx, x0, top, cw, height, r);
      ctx.clip();
      ctx.fillRect(x0, top, cw, title);
      if (cw > 24 && it.clip.name && title >= 10) {
        ctx.fillStyle = textColor;
        ctx.fillText(it.clip.name, x0 + 3, top + title / 2 + 0.5, cw - 6);
      }
      ctx.restore();
      if (sel) {
        ctx.strokeStyle = color;
        ctx.lineWidth = 1.5;
        roundRect(ctx, x0 + 0.75, top + 0.75, cw - 1.5, height - 1.5, r);
        ctx.stroke();
      }
    }
    ctx.globalAlpha = 1;
  });

  return (
    <canvas
      ref={ref}
      className="eth-arr-clusters"
      data-testid="small-clips"
      data-count={items.length}
      style={{ left: beatsCss(from - origin), width: beatsCss(to - from) }}
    />
  );
}

function roundRect(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number) {
  ctx.beginPath();
  if (r > 0 && ctx.roundRect) ctx.roundRect(x, y, w, h, r);
  else ctx.rect(x, y, w, h);
}
