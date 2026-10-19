/**
 * Velocity lane: one bar per note at its start. Dragging a bar vertically changes the
 * velocity of the selected notes (or just that note) by the same amount, as one gesture.
 *
 * The bars are drawn on a canvas (`noteLayout.ts` / `noteCanvas.ts`), like the note grid's
 * notes: presses are hit-tested against them (`hitVelocityBar`), and the cursor and the
 * "Velocity N" tooltip follow the bar under the pointer.
 */

import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useScopedDrag } from "@/kit";
import type { Note } from "@/generated";
import { t } from "@/i18n";
import { itemSelection, selectModeFromEvent, visibleRange, type TimelineViewport } from "@/timeline";
import { border, readToken, size, useThemeColor } from "@/theme";
import { parseColor } from "@/features/arrangement/gl/palette";
import { cmd, useTransport } from "@/transport";
import { startDrag } from "./drag";
import { VELOCITY_LANE_HEIGHT } from "./geometry";
import { velocityEdits } from "./noteEdits";
import { InstanceList, RectRenderer } from "./noteCanvas";
import { buildVelocityLayer, hitVelocityBar, registerScene, tagged, type SceneTag, type VelocityColors, type VelocityLaneInput } from "./noteLayout";

export interface VelocityLaneProps {
  notes: ReadonlyArray<Note>;
  vp: TimelineViewport;
  widthPx: number;
  height?: number;
}

/** Drawing width when the view's width is unknown (no layout, e.g. tests). */
const FALLBACK_WIDTH = 2000;

export function VelocityLane({ notes, vp, widthPx, height = VELOCITY_LANE_HEIGHT }: VelocityLaneProps) {
  const windowDrag = useScopedDrag(startDrag);
  const transport = useTransport();
  const rootRef = useRef<HTMLDivElement>(null);

  const visible = useMemo(() => {
    if (widthPx <= 0) return notes;
    const r = visibleRange(vp, widthPx);
    return notes.filter((n) => n.start <= r.end && n.start + n.duration >= r.start);
  }, [notes, vp, widthPx]);

  const accent = useThemeColor("accent");
  const selection = useThemeColor("selection");
  const text = useThemeColor("text");
  const colors: VelocityColors = useMemo(
    () => ({ bar: parseColor(accent), selected: parseColor(selection), outline: parseColor(text) }),
    [accent, selection, text],
  );
  const metrics = useMemo(() => {
    const px = (name: `--${string}`, fallback: string) => parseFloat(readToken(name) || fallback) || parseFloat(fallback);
    return { barWidth: px("--eth-size-velocity-bar-width", size.velocityBarWidth), borderWidth: px("--eth-border-width", border.width) };
  }, []);
  const width = widthPx > 0 ? widthPx : FALLBACK_WIDTH;
  const input = (): VelocityLaneInput => ({ notes: visible, selected: itemSelection.getState().selected.note, vp, height, width, ...metrics });

  const onPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    const box = e.currentTarget.getBoundingClientRect();
    const note = e.target === e.currentTarget ? hitVelocityBar(e.clientX - box.left, e.clientY - box.top, input()) : null;
    if (!note) {
      if (selectModeFromEvent(e) === "replace") itemSelection.getState().clear("note");
      return;
    }
    const sel = itemSelection.getState();
    if (!sel.selected.note.has(note.id)) {
      const mode = selectModeFromEvent(e);
      sel.select("note", [note.id], mode === "remove" ? "replace" : mode);
    }
    const selected = itemSelection.getState().selected.note;
    const originals = notes.filter((n) => selected.has(n.id));
    windowDrag(transport, e, { move: (_dx, dy) => cmd("Note", { type: "Edit", edits: velocityEdits(originals, -dy / height) }) }, { cursor: "ns-resize" });
  };

  /** Over a bar: the resize cursor and its velocity as the tooltip. */
  const onPointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    const el = e.currentTarget;
    const box = el.getBoundingClientRect();
    const note = hitVelocityBar(e.clientX - box.left, e.clientY - box.top, input());
    const cursor = note ? "ns-resize" : "";
    if (el.style.cursor !== cursor) el.style.cursor = cursor;
    const title = note ? t("piano_roll.velocity", { value: Math.round(note.velocity * 127) }) : "";
    if (el.title !== title) el.title = title;
  };

  // ---- Drawing: like the note grid (a render draws at once; selection changes per frame) ----
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const renderer = useRef<RectRenderer | null>(null);
  const [list] = useState(() => new InstanceList());
  const drawRef = useRef(() => {});
  useLayoutEffect(() => {
    drawRef.current = () => {
      const canvas = canvasRef.current;
      if (!canvas) return;
      renderer.current ??= new RectRenderer(canvas);
      const w = `${width}px`;
      const h = `${height}px`;
      if (canvas.style.width !== w) canvas.style.width = w;
      if (canvas.style.height !== h) canvas.style.height = h;
      list.clear();
      buildVelocityLayer(input(), colors, list);
      renderer.current.draw(list, width, height);
    };
  });
  const frameId = useRef(0);
  useLayoutEffect(() => {
    cancelAnimationFrame(frameId.current);
    frameId.current = 0;
    drawRef.current();
  });
  useEffect(() => {
    let selected = itemSelection.getState().selected.note;
    const unsubscribe = itemSelection.subscribe((s) => {
      if (s.selected.note === selected) return;
      selected = s.selected.note;
      if (frameId.current) return;
      frameId.current = requestAnimationFrame(() => {
        frameId.current = 0;
        drawRef.current();
      });
    });
    return () => {
      unsubscribe();
      cancelAnimationFrame(frameId.current);
      frameId.current = 0;
      renderer.current?.dispose();
      renderer.current = null;
    };
  }, []);
  // Tests and debugging: what the canvas draws (`velocityScene`).
  const inputRef = useRef(input);
  useLayoutEffect(() => {
    inputRef.current = input;
  });
  useLayoutEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    return registerScene(el, () => {
      const out = new InstanceList();
      const tags: SceneTag[] = [];
      buildVelocityLayer(inputRef.current(), colors, out, tags);
      return tagged(out, tags);
    });
  }, [colors]);

  return (
    <div
      ref={rootRef}
      className="eth-pr-velocity"
      style={{ height }}
      data-testid="piano-roll-velocity"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
    >
      {/* Pixels only: the notes' velocities are in the piano roll's NoteSummary, as text. */}
      <canvas ref={canvasRef} className="eth-pr-velocity__canvas" data-testid="piano-roll-velocity-canvas" aria-hidden />
    </div>
  );
}
