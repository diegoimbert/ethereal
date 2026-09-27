/**
 * Velocity lane: one bar per note at its start. Dragging a bar vertically changes the
 * velocity of the selected notes (or just that note) by the same amount, as one gesture.
 */

import { useMemo } from "react";
import clsx from "clsx";
import type { Note } from "@/generated";
import { beatsToPx, itemSelection, selectModeFromEvent, useIsSelected, visibleRange, type TimelineViewport } from "@/timeline";
import { cmd, useTransport } from "@/transport";
import { startDrag } from "./drag";
import { VELOCITY_LANE_HEIGHT } from "./geometry";
import { velocityEdits } from "./noteEdits";

export interface VelocityLaneProps {
  notes: ReadonlyArray<Note>;
  vp: TimelineViewport;
  widthPx: number;
  height?: number;
}

export function VelocityLane({ notes, vp, widthPx, height = VELOCITY_LANE_HEIGHT }: VelocityLaneProps) {
  const transport = useTransport();

  const visible = useMemo(() => {
    if (widthPx <= 0) return notes;
    const r = visibleRange(vp, widthPx);
    return notes.filter((n) => n.start <= r.end && n.start + n.duration >= r.start);
  }, [notes, vp, widthPx]);

  const onBarPointerDown = (e: React.PointerEvent<HTMLDivElement>, note: Note) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    const sel = itemSelection.getState();
    if (!sel.selected.note.has(note.id)) {
      const mode = selectModeFromEvent(e);
      sel.select("note", [note.id], mode === "remove" ? "replace" : mode);
    }
    const selected = itemSelection.getState().selected.note;
    const originals = notes.filter((n) => selected.has(n.id));
    startDrag(
      transport,
      e,
      { move: (_dx, dy) => cmd("Note", { type: "Edit", edits: velocityEdits(originals, -dy / height) }) },
      { cursor: "ns-resize" },
    );
  };

  return (
    <div
      className="eth-pr-velocity"
      style={{ height }}
      data-testid="piano-roll-velocity"
      onPointerDown={(e) => {
        if (e.button === 0 && selectModeFromEvent(e) === "replace") itemSelection.getState().clear("note");
      }}
    >
      {visible.map((n) => (
        <VelocityBar key={n.id} note={n} x={beatsToPx(n.start, vp)} height={height} onPointerDown={onBarPointerDown} />
      ))}
    </div>
  );
}

function VelocityBar({
  note,
  x,
  height,
  onPointerDown,
}: {
  note: Note;
  x: number;
  height: number;
  onPointerDown: (e: React.PointerEvent<HTMLDivElement>, note: Note) => void;
}) {
  const selected = useIsSelected("note", note.id);
  const h = Math.max(2, note.velocity * (height - 4));
  return (
    <div
      className={clsx("eth-pr-vel", selected && "eth-pr-vel--selected")}
      data-testid="piano-roll-velocity-bar"
      data-note-id={note.id}
      style={{ transform: `translateX(${Math.round(x)}px)`, height: h }}
      title={`Velocity ${Math.round(note.velocity * 127)}`}
      onPointerDown={(e) => onPointerDown(e, note)}
    />
  );
}
