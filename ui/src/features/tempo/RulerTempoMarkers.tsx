/**
 * Tempo-map editing on the timeline ruler (rendered by `timeline/Ruler.tsx`): markers for
 * tempo points and time-signature changes along the ruler's bottom edge.
 *
 * - drag a marker to move it (tempo: grid-snapped, Alt bypasses; signature: along the bar
 *   lines of the signature before it); the ones at beat 0 stay. One undo step per drag;
 * - right-click a marker: ramp on/off or common signatures, delete;
 * - right-click the ruler: add a tempo change or a time signature there
 *   (`rulerTempoMenu` in `menus.ts`).
 */

import { useLayoutEffect, useRef, type PointerEvent as ReactPointerEvent } from "react";
import type { Beats, Command, TempoPoint, TimeSignaturePoint } from "@/generated";
import { openContextMenu, setDragCursor } from "@/kit";
import { beatsToPx, type TimelineViewport } from "@/timeline/viewport";
import { sendEdit, TempoGesture, trackDrag, useTempoTransport } from "./gesture";
import { signatureMenu, useSortedTempoMap } from "./menus";
import {
  editSignatureCommand,
  editTempoPointCommand,
  formatBpm,
  formatSignature,
  isAtZero,
  removeSignaturesCommand,
  removeTempoPointsCommand,
  snapSignatureTime,
  tempoTimeTaken,
} from "./tempoCommands";
import "./tempo.css";

export interface RulerTempoMarkersProps {
  vp: TimelineViewport;
  widthPx: number;
  /** Snap a raw beat position (the ruler's grid; `bypass` = Alt). */
  snap: (beats: Beats, bypass: boolean) => Beats;
}

export function RulerTempoMarkers({ vp, widthPx, snap }: RulerTempoMarkersProps) {
  const transport = useTempoTransport();
  const { points, signatures } = useSortedTempoMap();
  const live = useRef({ points, signatures, vp });
  useLayoutEffect(() => {
    live.current = { points, signatures, vp };
  });

  const visible = (b: Beats) => {
    const x = beatsToPx(b, vp);
    return x >= -40 && x <= widthPx + 4;
  };

  /** Drag a marker horizontally; `place` maps a raw position to a valid one (or null). */
  const startDrag = (e: ReactPointerEvent, time: Beats, place: (raw: Beats, alt: boolean) => Beats | null, command: (t: Beats) => Command) => {
    const startX = e.clientX;
    const gesture = new TempoGesture(transport);
    let last = time;
    trackDrag(
      e,
      (ev) => {
        setDragCursor("grabbing");
        const t = place(time + (ev.clientX - startX) / live.current.vp.pxPerBeat, ev.altKey);
        if (t === null || t === last) return;
        last = t;
        gesture.update(command(t));
      },
      () => {
        setDragCursor(null);
        gesture.end();
      },
    );
  };

  const onTempoDown = (e: ReactPointerEvent, p: TempoPoint) => {
    e.stopPropagation();
    if (e.button !== 0 || isAtZero(p)) return;
    e.preventDefault();
    startDrag(e, p.time, (raw, alt) => {
      const t = Math.max(0, snap(raw, alt));
      return tempoTimeTaken(live.current.points, t, p.id) ? null : t;
    }, (t) => editTempoPointCommand(p.id, { time: t }));
  };

  const onSigDown = (e: ReactPointerEvent, p: TimeSignaturePoint) => {
    e.stopPropagation();
    if (e.button !== 0 || isAtZero(p)) return;
    e.preventDefault();
    startDrag(e, p.time, (raw) => snapSignatureTime(live.current.signatures, raw, p.id), (t) =>
      editSignatureCommand(p.id, { time: t }),
    );
  };

  // One stop per position: a signature change and a tempo change at the same beat sit
  // side by side instead of overlapping.
  const stops = new Map<number, { time: Beats; sig?: TimeSignaturePoint; tp?: TempoPoint }>();
  const key = (t: Beats) => Math.round(t * 1e4);
  for (const s of signatures) if (visible(s.time)) stops.set(key(s.time), { time: s.time, sig: s });
  for (const p of points) {
    if (!visible(p.time)) continue;
    const stop = stops.get(key(p.time));
    if (stop) stop.tp = p;
    else stops.set(key(p.time), { time: p.time, tp: p });
  }

  return (
    <div className="eth-ruler-tempo" data-testid="ruler-tempo">
      {[...stops.entries()].map(([k, { time, sig, tp }]) => (
        <div key={k} className="eth-ruler-tempo__stop" style={{ transform: `translateX(${Math.round(beatsToPx(time, vp))}px)` }}>
          {sig && (
            <div
              className="eth-ruler-tempo__marker eth-ruler-tempo__marker--signature"
              data-signature={sig.id}
              title={isAtZero(sig) ? "Time signature" : "Time signature (drag to move)"}
              onPointerDown={(e) => onSigDown(e, sig)}
              onDoubleClick={(e) => e.stopPropagation()}
              onContextMenu={(e) =>
                openContextMenu(
                  e,
                  signatureMenu(
                    sig,
                    (s) => void sendEdit(transport, editSignatureCommand(sig.id, { signature: s })),
                    () => void sendEdit(transport, removeSignaturesCommand([sig.id])),
                  ),
                )
              }
            >
              {formatSignature(sig.signature)}
            </div>
          )}
          {tp && (
            <div
              className="eth-ruler-tempo__marker eth-ruler-tempo__marker--tempo"
              data-tempo-point={tp.id}
              title={isAtZero(tp) ? "Tempo" : "Tempo change (drag to move)"}
              onPointerDown={(e) => onTempoDown(e, tp)}
              onDoubleClick={(e) => e.stopPropagation()}
              onContextMenu={(e) =>
                openContextMenu(e, [
                  {
                    label: tp.curve === "Linear" ? "Hold Tempo (No Ramp)" : "Ramp to Next Point",
                    onSelect: () =>
                      void sendEdit(transport, editTempoPointCommand(tp.id, { curve: tp.curve === "Linear" ? "Step" : "Linear" })),
                  },
                  "separator",
                  {
                    label: "Delete Tempo Change",
                    danger: true,
                    disabled: isAtZero(tp),
                    onSelect: () => void sendEdit(transport, removeTempoPointsCommand([tp.id])),
                  },
                ])
              }
            >
              {`♩${formatBpm(tp.bpm)}${tp.curve === "Linear" ? " ↗" : ""}`}
            </div>
          )}
        </div>
      ))}
    </div>
  );
}
