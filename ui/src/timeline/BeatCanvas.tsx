import { useLayoutEffect, useRef } from "react";
import type { Beats } from "@/generated";
import { settledZoom, subscribeSettledZoom } from "./settledZoom";
import type { TimelineViewStore } from "./viewStore";
import { renderDpr } from "./renderDpr";

/** What a `BeatCanvas` draws: a beat range at a zoom, on a canvas `width` × `height` CSS px. */
export interface BeatWindow {
  from: Beats;
  to: Beats;
  /** Pixels per beat the window is drawn at (the settled zoom). */
  ppb: number;
  width: number;
  height: number;
}

export interface BeatCanvasProps {
  view: TimelineViewStore;
  /** Drawing height (CSS px). With `stretchY`, the canvas is drawn 1px tall and stretched. */
  height: number;
  /** Draw a single row and stretch it to the container's height (vertical lines). */
  stretchY?: boolean;
  /** Draw `win` (x = (beats - win.from) * win.ppb). The context is cleared and scaled. */
  draw: (ctx: CanvasRenderingContext2D | null, win: BeatWindow, canvas: HTMLCanvasElement) => void;
  /** Redraw when one of these changes (besides the view leaving the drawn window). */
  deps: ReadonlyArray<unknown>;
  className?: string;
  testId?: string;
}

/** Widest canvas drawn (device px). */
const MAX_CANVAS_PX = 16384;

/**
 * A canvas of timeline content (ruler marks, grid lines) that follows pan and zoom without
 * repainting: it draws a window three viewports wide at the settled zoom, and each view
 * change only moves and stretches it with a composited transform
 * (`translateX(…) scaleX(live / settled)`). It redraws when the view leaves the window, the
 * zoom settles, or `deps` change. Repainting a canvas on every frame forced the browser to
 * re-layerize the whole page (a cost that grows with everything else on screen).
 */
export function BeatCanvas({ view, height, stretchY, draw, deps, className, testId }: BeatCanvasProps) {
  const ref = useRef<HTMLCanvasElement>(null);
  const drawRef = useRef(draw);
  // The latest `draw` for redraws triggered by the view (declared first: runs before the
  // effect below on the same commit).
  useLayoutEffect(() => {
    drawRef.current = draw;
  });

  useLayoutEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    let win: BeatWindow | null = null;
    const redraw = (scroll: Beats, settled: number, viewport: number) => {
      const dpr = renderDpr();
      const span = viewport / settled;
      let from = Math.max(0, scroll - span);
      let to = scroll + 2 * span;
      if ((to - from) * settled * dpr > MAX_CANVAS_PX) {
        const maxBeats = MAX_CANVAS_PX / (settled * dpr);
        from = Math.max(0, scroll - (maxBeats - span) / 2);
        to = from + maxBeats;
      }
      const width = Math.max(1, Math.round((to - from) * settled));
      const pw = Math.max(1, Math.round(width * dpr));
      const ph = stretchY ? 1 : Math.max(1, Math.round(height * dpr));
      if (canvas.width !== pw) canvas.width = pw;
      if (canvas.height !== ph) canvas.height = ph;
      if (canvas.style.width !== `${width}px`) canvas.style.width = `${width}px`;
      const h = stretchY ? "100%" : `${height}px`;
      if (canvas.style.height !== h) canvas.style.height = h;
      win = { from, to, ppb: settled, width, height: stretchY ? 1 : height };
      const ctx = canvas.getContext("2d");
      if (ctx) {
        ctx.setTransform(pw / width, 0, 0, stretchY ? 1 : dpr, 0, 0);
        ctx.clearRect(0, 0, width, win.height);
      }
      // `draw` also runs without a context (tests: it may publish test hooks).
      drawRef.current(ctx, win, canvas);
    };
    const update = () => {
      const s = view.getState();
      const settled = settledZoom(view);
      const viewport = s.widthPx || canvas.parentElement?.clientWidth || 1000;
      const end = s.scrollBeats + viewport / s.pxPerBeat;
      if (!win || win.ppb !== settled || s.scrollBeats < win.from - 1e-9 || end > win.to + 1e-9) {
        redraw(s.scrollBeats, settled, viewport);
      }
      const w = win!;
      const scale = s.pxPerBeat / settled;
      const x = (w.from - s.scrollBeats) * s.pxPerBeat;
      canvas.style.transform = scale === 1 ? `translateX(${x}px)` : `translateX(${x}px) scaleX(${scale})`;
    };
    update();
    const offView = view.subscribe(update);
    const offSettled = subscribeSettledZoom(view, update);
    return () => {
      offView();
      offSettled();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- `deps` are the draw inputs
  }, [view, height, stretchY, ...deps]);

  // Decorative (grid lines, ruler ticks): the ruler's own label and value say where the view is.
  return <canvas ref={ref} className={className ? `eth-beat-canvas ${className}` : "eth-beat-canvas"} data-testid={testId} aria-hidden />;
}
