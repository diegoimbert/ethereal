import "./clipEditing.css";
import { useRef, type PointerEvent as ReactPointerEvent } from "react";
import type { Clip, FadeCurve } from "@/generated";
import { Badge, setDragCursor } from "@/kit";
import type { EngineTransport } from "@/transport";
import { LaneGesture } from "@/features/automation/gesture";
import { cmd } from "@/transport";
import { audioOf, dragCurve, dragFadeLength, fadePaths, sendEdit } from "./clipEditing";
import { fadeGain } from "./fades";

export interface ClipFadesProps {
  clip: Clip;
  /** Clip length on screen (beats; the live drag preview while resizing). */
  length: number;
  pxPerBeat: number;
  transport: EngineTransport;
}

type Side = "in" | "out";

/**
 * Fade overlay of an audio clip (inside the clip body zone): the fade shapes drawn with the
 * fade law (`fadeGain`), a length handle per fade (drag horizontally) and a curve handle at
 * the fade's midpoint (drag vertically to bend it; double-click resets it to linear). Each
 * drag is one undo step.
 */
export function ClipFades({ clip, length, pxPerBeat, transport }: ClipFadesProps) {
  const ref = useRef<HTMLDivElement>(null);
  const a = audioOf(clip);
  if (!a) return null;
  const width = Math.max(1, length * pxPerBeat);
  const fin = Math.min(a.fade_in, length) * pxPerBeat;
  const fout = Math.min(a.fade_out, length) * pxPerBeat;
  const paths = { in: fadePaths("in", fin, width, a.fade_in_curve), out: fadePaths("out", fout, width, a.fade_out_curve) };
  // Handles as a share of the clip's width: the arrangement stretches clips with CSS while
  // zooming (before re-rendering at the settled zoom), and the SVG stretches with them.
  const pct = (px: number) => `${((px / width) * 100).toFixed(3)}%`;

  const startLengthDrag = (side: Side) => (e: ReactPointerEvent<HTMLElement>) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    e.preventDefault();
    const gesture = new LaneGesture(transport);
    const x0 = e.clientX;
    const [start, other] = side === "in" ? [a.fade_in, a.fade_out] : [a.fade_out, a.fade_in];
    setDragCursor("ew-resize");
    const move = (ev: PointerEvent) => {
      const v = dragFadeLength(side, start, ev.clientX - x0, pxPerBeat, clip.length, other);
      const [fade_in, fade_out] = side === "in" ? [v, a.fade_out] : [a.fade_in, v];
      gesture.update(cmd("Clip", { type: "SetFades", id: clip.id, fade_in, fade_out }));
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
      setDragCursor(null);
      gesture.end();
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
  };

  const curveCommand = (side: Side, curve: FadeCurve) =>
    cmd("Clip", {
      type: "SetFadeCurves",
      id: clip.id,
      fade_in: side === "in" ? curve : null,
      fade_out: side === "out" ? curve : null,
    });

  const startCurveDrag = (side: Side) => (e: ReactPointerEvent<HTMLElement>) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    e.preventDefault();
    const height = ref.current?.clientHeight || 1;
    const gesture = new LaneGesture(transport);
    const y0 = e.clientY;
    const start = side === "in" ? a.fade_in_curve : a.fade_out_curve;
    setDragCursor("ns-resize");
    const move = (ev: PointerEvent) => gesture.update(curveCommand(side, dragCurve(start, (ev.clientY - y0) / height)));
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
      setDragCursor(null);
      gesture.end();
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
  };

  const curveHandle = (side: Side, fadePx: number, curve: FadeCurve) => {
    if (fadePx < 8) return null;
    const x = side === "in" ? fadePx / 2 : width - fadePx / 2;
    const top = `${((1 - fadeGain(curve, 0.5)) * 100).toFixed(2)}%`;
    return (
      <div
        className="eth-clip-fades__curve"
        data-handle={`fade-${side}-curve`}
        data-testid={`fade-${side}-curve`}
        title="Drag to bend the fade (double-click: linear)"
        style={{ left: pct(x), top }}
        onPointerDown={startCurveDrag(side)}
        onDoubleClick={(e) => {
          e.stopPropagation();
          void sendEdit(transport, curveCommand(side, { type: "Linear" }));
        }}
      />
    );
  };

  return (
    <div ref={ref} className="eth-clip-fades" data-testid="clip-fades" data-fade-in={a.fade_in} data-fade-out={a.fade_out}>
      <svg className="eth-clip-fades__svg" viewBox={`0 0 ${width} 1`} preserveAspectRatio="none" aria-hidden>
        {(["in", "out"] as const).map((side) => {
          const p = paths[side];
          return p ? (
            <g key={side}>
              <path className="eth-clip-fades__shade" d={p.shade} />
              <path className="eth-clip-fades__line" d={p.line} />
            </g>
          ) : null;
        })}
      </svg>
      <div
        className="eth-clip-fades__handle eth-clip-fades__handle--in"
        data-handle="fade-in"
        data-testid="fade-in-handle"
        title="Fade in"
        style={{ left: `max(${pct(fin)}, var(--eth-space-md))` }}
        onPointerDown={startLengthDrag("in")}
      />
      <div
        className="eth-clip-fades__handle eth-clip-fades__handle--out"
        data-handle="fade-out"
        data-testid="fade-out-handle"
        title="Fade out"
        style={{ left: `min(${pct(width - fout)}, calc(100% - var(--eth-space-md)))` }}
        onPointerDown={startLengthDrag("out")}
      />
      {curveHandle("in", fin, a.fade_in_curve)}
      {curveHandle("out", fout, a.fade_out_curve)}
    </div>
  );
}

/** "Reversed" badge in the clip title. */
export function ReversedBadge() {
  return (
    <span className="eth-clip-reversed" title="Reversed" data-testid="clip-reversed">
      <Badge tone="accent">REV</Badge>
    </span>
  );
}
