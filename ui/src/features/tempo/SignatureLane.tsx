/**
 * Time-signature lane of the tempo editor: one marker per signature change.
 *
 * - drag a marker: it moves anywhere on the grid (a beat of the signature in effect, finer
 *   if the grid is finer; Alt bypasses snapping), one undo step; the one at beat 0 stays;
 * - double-click the lane: add a change there, snapped the same way (same signature, edit
 *   it in the header or from the marker's menu); double-click a marker: remove it;
 * - right-click a marker: common signatures, delete.
 * A change inside a bar ends that bar early (CONTRACTS.md §11.3).
 */

import { useLayoutEffect, useRef, type KeyboardEvent, type MouseEvent, type PointerEvent as ReactPointerEvent } from "react";
import type { TimeSignaturePoint } from "@/generated";
import { openContextMenu, setDragCursor } from "@/kit";
import { newId } from "@/transport";
import { DEFAULT_GRID, resolveGrid } from "@/timeline/grid";
import type { TempoMap } from "@/timeline/tempoMap";
import { beatsToPx, pxToBeats } from "@/timeline/viewport";
import { useViewport, type TimelineViewStore } from "@/timeline/viewStore";
import { sendEdit, TempoGesture, trackDrag, useTempoTransport } from "./gesture";
import { signatureMenu } from "./menus";
import {
  addSignatureCommand,
  editSignatureCommand,
  formatSignature,
  isAtZero,
  removeSignaturesCommand,
  snapSignatureTime,
} from "./tempoCommands";

export interface SignatureLaneProps {
  view: TimelineViewStore;
  tempo: TempoMap;
  signatures: TimeSignaturePoint[];
  selected: string | null;
  onSelect: (id: string | null) => void;
}

export function SignatureLane({ view, tempo, signatures, selected, onSelect }: SignatureLaneProps) {
  const transport = useTempoTransport();
  const vp = useViewport(view);
  const rootRef = useRef<HTMLDivElement>(null);
  const live = useRef(signatures);
  useLayoutEffect(() => {
    live.current = signatures;
  });

  const localX = (e: { clientX: number }) => e.clientX - (rootRef.current?.getBoundingClientRect().left ?? 0);
  /** The lane's grid step (adaptive, as in the tempo lane). */
  const gridStep = () => {
    const st = view.getState();
    return resolveGrid(DEFAULT_GRID, st.pxPerBeat, tempo.signatureAt(st.scrollBeats));
  };

  const remove = (p: TimeSignaturePoint) => {
    if (isAtZero(p)) return;
    if (selected === p.id) onSelect(null);
    void sendEdit(transport, removeSignaturesCommand([p.id]));
  };

  const onMarkerPointerDown = (e: ReactPointerEvent, p: TimeSignaturePoint) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    e.preventDefault();
    rootRef.current?.focus();
    onSelect(p.id);
    if (isAtZero(p)) return;
    const startX = localX(e);
    const gesture = new TempoGesture(transport);
    let last = p.time;
    trackDrag(
      e,
      (ev) => {
        setDragCursor("grabbing");
        const raw = p.time + (localX(ev) - startX) / view.getState().pxPerBeat;
        const t = snapSignatureTime(live.current, raw, { except: p.id, step: gridStep(), free: ev.altKey });
        if (t === null || t === last) return;
        last = t;
        gesture.update(editSignatureCommand(p.id, { time: t }));
      },
      () => {
        setDragCursor(null);
        gesture.end();
      },
    );
  };

  const onDoubleClick = (e: MouseEvent<HTMLDivElement>) => {
    e.stopPropagation();
    if (e.target !== e.currentTarget) return;
    const raw = pxToBeats(localX(e), view.getState());
    const t = snapSignatureTime(live.current, raw, { step: gridStep(), free: e.altKey });
    if (t === null) return;
    const id = newId();
    void sendEdit(transport, addSignatureCommand(t, tempo.signatureAt(t), id)).then(() => onSelect(id));
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== "Delete" && e.key !== "Backspace") return;
    e.preventDefault();
    e.stopPropagation();
    const p = live.current.find((s) => s.id === selected);
    if (p) remove(p);
  };

  return (
    <div
      ref={rootRef}
      className="eth-tempo-sigs"
      tabIndex={0}
      role="group"
      aria-label="Time signature changes"
      onPointerDown={(e) => {
        e.stopPropagation();
        if (e.button === 0) onSelect(null);
      }}
      onDoubleClick={onDoubleClick}
      onKeyDown={onKeyDown}
      data-testid="signature-lane"
    >
      {signatures.map((p) => {
        const on = p.id === selected;
        return (
          <div
            key={p.id}
            className={on ? "eth-tempo-sigs__marker eth-tempo-sigs__marker--selected" : "eth-tempo-sigs__marker"}
            style={{ transform: `translateX(${Math.round(beatsToPx(p.time, vp))}px)` }}
            data-signature={p.id}
            aria-selected={on}
            title={isAtZero(p) ? "Time signature" : "Time signature (drag to move, double-click to remove)"}
            onPointerDown={(e) => onMarkerPointerDown(e, p)}
            onDoubleClick={(e) => {
              e.stopPropagation();
              remove(p);
            }}
            onContextMenu={(e) => {
              onSelect(p.id);
              openContextMenu(
                e,
                signatureMenu(
                  p,
                  (s) => void sendEdit(transport, editSignatureCommand(p.id, { signature: s })),
                  () => remove(p),
                ),
              );
            }}
          >
            {formatSignature(p.signature)}
          </div>
        );
      })}
    </div>
  );
}
