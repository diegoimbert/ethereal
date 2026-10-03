/**
 * The note-stretch bar on the piano roll's ruler and its ×2 / ÷2 buttons (base-109; the
 * math and the behaviour are in `stretch.ts`). Hovering an edge shows a resize cursor, the
 * body a grab cursor (like the arrangement's loop brace). A drag is one undo gesture whose
 * commands are applied live (the preview is the edit itself).
 */

import { useMemo, useState, type PointerEvent as ReactPointerEvent } from "react";
import type { Beats, Clip, Note } from "@/generated";
import { Button } from "@/kit";
import { beatsToPx, snapToGrid, useSelectedItems, type GridStep, type TempoMap, type TimelineViewport } from "@/timeline";
import { useTransport } from "@/transport";
import { startDrag, useSend } from "./drag";
import { usePianoRollSection } from "./section";
import {
  scaledRange,
  stretchCommand,
  stretchedSection,
  stretchHandleAt,
  stretchRange,
  stretchSource,
  type StretchHandle,
  type StretchRange,
  type StretchSource,
} from "./stretch";

const HOVER_CURSOR: Record<StretchHandle, string> = { start: "ew-resize", end: "ew-resize", move: "grab" };
const DRAG_CURSOR: Record<StretchHandle, string> = { start: "ew-resize", end: "ew-resize", move: "grabbing" };

/** What the bar spans in `clip` (null: no bar). */
export function useStretchSource(clip: Clip, notes: ReadonlyArray<Note>): StretchSource | null {
  const selected = useSelectedItems("note");
  const section = usePianoRollSection((s) => (s.section?.clip === clip.id ? s.section : null));
  return useMemo(() => stretchSource(notes, selected, section), [notes, selected, section]);
}

export interface StretchBarProps {
  clip: Clip;
  notes: ReadonlyArray<Note>;
  vp: TimelineViewport;
  step: GridStep | null;
  tempo: TempoMap;
}

export function StretchBar({ clip, notes, vp, step, tempo }: StretchBarProps) {
  const transport = useTransport();
  const source = useStretchSource(clip, notes);
  // While dragging, the bar shows the drag's range (the notes follow as patches arrive).
  const [dragRange, setDragRange] = useState<StretchRange | null>(null);
  const range = dragRange ?? source;
  if (!range) return null;

  const onPointerDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (e.button !== 0 || !source) return;
    e.preventDefault();
    e.stopPropagation();
    const box = e.currentTarget.getBoundingClientRect();
    const handle = stretchHandleAt(e.clientX - box.left, box.width);
    const from = source;
    const original = clip;
    const pxPerBeat = vp.pxPerBeat;
    let grown = false;
    startDrag(
      transport,
      e,
      {
        move(dx, _dy, m) {
          const snap = (b: Beats) => snapToGrid(b, m.altKey ? null : step, tempo, "nearest");
          const to = stretchRange(from, handle, dx / pxPerBeat, snap);
          setDragRange(to);
          const section = stretchedSection(original, from, to);
          if (section) usePianoRollSection.getState().setSection(section);
          const { command, grows } = stretchCommand(original, from, to, grown);
          grown ||= grows;
          return command;
        },
        end() {
          setDragRange(null);
        },
      },
      { cursor: DRAG_CURSOR[handle] },
    );
  };

  /** Resize cursor over the edges (the zones the drag uses), grab over the body. */
  const onPointerMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    const box = e.currentTarget.getBoundingClientRect();
    const cursor = HOVER_CURSOR[stretchHandleAt(e.clientX - box.left, box.width)];
    if (e.currentTarget.style.cursor !== cursor) e.currentTarget.style.cursor = cursor;
  };

  return (
    <div className="eth-pr-stretch-layer">
      <div
        className={dragRange ? "eth-pr-stretch eth-pr-stretch--dragging" : "eth-pr-stretch"}
        data-testid="piano-roll-stretch"
        style={{ left: beatsToPx(range.start, vp), width: (range.end - range.start) * vp.pxPerBeat }}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        title="Stretch the selected notes (drag an edge), move them (drag the bar); Alt: no snap"
      >
        <div className="eth-pr-stretch__edge eth-pr-stretch__edge--start" data-testid="piano-roll-stretch-start" />
        <div className="eth-pr-stretch__edge eth-pr-stretch__edge--end" data-testid="piano-roll-stretch-end" />
      </div>
    </div>
  );
}

/** ×2 / ÷2: double or halve the selection's length about its start (one undo step each). */
export function StretchButtons({ clip, notes }: { clip: Clip; notes: ReadonlyArray<Note> }) {
  const send = useSend();
  const source = useStretchSource(clip, notes);
  const scale = (factor: number) => {
    if (!source) return;
    const to = scaledRange(source, factor);
    const section = stretchedSection(clip, source, to);
    void send(stretchCommand(clip, source, to).command).then((ok) => {
      if (ok && section) usePianoRollSection.getState().setSection(section);
    });
  };
  return (
    <>
      <Button size="sm" disabled={!source} onClick={() => scale(2)} title="Double the selected notes' length (×2)">
        ×2
      </Button>
      <Button size="sm" disabled={!source} onClick={() => scale(0.5)} title="Halve the selected notes' length (÷2)">
        ÷2
      </Button>
    </>
  );
}
