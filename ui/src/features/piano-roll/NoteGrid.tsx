/**
 * The note grid: pitch rows, grid lines, the clip's playable region, notes and the
 * marquee. Draw / move / resize notes; every drag is one undo gesture.
 */

import { memo, useMemo, useRef, type PointerEvent as ReactPointerEvent } from "react";
import clsx from "clsx";
import { scaleTone } from "@/domain/scales";
import type { MusicalScale, Clip, Note, NoteId } from "@/generated";
import {
  beatsToPx,
  gridLines,
  itemSelection,
  marqueeHits,
  PlayheadLine,
  pxToBeats,
  selectModeFromEvent,
  snapToGrid,
  useIsSelected,
  useMarquee,
  visibleRange,
  type GridStep,
  type TempoMap,
  type TimelineViewport,
  type TimelineViewStore,
} from "@/timeline";
import { cmd, newId, useTransport } from "@/transport";
import { contentEnd, songToContent } from "./clipTime";
import { startDrag, useSend } from "./drag";
import { isBlackKey, noteHitZone, noteRect, pitchToY, yToPitch } from "./geometry";
import { moveEdits, newNote, noteEdit, resizeEdits } from "./noteEdits";

export interface NoteGridProps {
  clip: Clip;
  notes: ReadonlyArray<Note>;
  view: TimelineViewStore;
  vp: TimelineViewport;
  widthPx: number;
  keyH: number;
  rows: readonly number[];
  scale: MusicalScale;
  highlight: boolean;
  tempo: TempoMap;
  /** Snap step (`null` = grid off). Alt bypasses snapping during a drag. */
  step: GridStep | null;
  /** Length of a new note (the grid step, or a 1/16 when the grid is off). */
  newNoteBeats: number;
  drawMode: boolean;
}

const BAR_STEP: GridStep = { kind: "bars", bars: 1 };

export function NoteGrid({ clip, notes, view, vp, widthPx, keyH, tempo, step, newNoteBeats, drawMode, rows, scale, highlight }: NoteGridProps) {
  const transport = useTransport();
  const send = useSend();
  const rootRef = useRef<HTMLDivElement>(null);
  const height = rows.length * keyH;

  const range = useMemo(() => visibleRange(vp, widthPx > 0 ? widthPx : 2000), [vp, widthPx]);
  const lines = useMemo(() => gridLines(tempo, range, step ?? BAR_STEP), [tempo, range, step]);
  const visible = useMemo(
    () =>
      widthPx > 0
        ? notes.filter((n) => n.start <= range.end && n.start + n.duration >= range.start)
        : notes,
    [notes, range, widthPx],
  );

  const marquee = useMarquee({
    kind: "note",
    hitTest: (rect) => marqueeHits(rect, notes.map((n) => ({ id: n.id, rect: noteRect(n, vp, keyH, rows) }))),
  });

  const local = (e: { clientX: number; clientY: number }) => {
    const box = rootRef.current!.getBoundingClientRect();
    return { x: e.clientX - box.left, y: e.clientY - box.top };
  };

  /** Add a note at the pointer; with `drag`, the drag extends it (one gesture). */
  const addNoteAt = (e: ReactPointerEvent | React.MouseEvent, drag: boolean) => {
    const { x, y } = local(e);
    const snap = e.altKey ? null : step;
    const start = snapToGrid(pxToBeats(x, vp), snap, tempo, "floor");
    const spec = newNote(newId(), yToPitch(y, keyH, rows), start, newNoteBeats);
    const add = cmd("Note", { type: "Add", clip: clip.id, notes: [spec] });
    // Select once the note exists (the selection is pruned against the project).
    const selectIt = () => itemSelection.getState().select("note", [spec.id], "replace");
    if (!drag) {
      void send(add).then((ok) => ok && selectIt());
      return;
    }
    const pressBeats = pxToBeats(x, vp);
    startDrag(
      transport,
      e,
      {
        // The note extends to cover the grid cell under the pointer.
        move: (dx, _dy, ev) => {
          const pointer = pressBeats + dx / vp.pxPerBeat;
          const end = snapToGrid(pointer, ev.altKey ? null : step, tempo, "ceil");
          const duration = Math.max(spec.duration, end - spec.start);
          return cmd("Note", { type: "Edit", edits: [noteEdit(spec.id, { duration })] });
        },
      },
      { initial: add, afterInitial: selectIt, threshold: 3 },
    );
  };

  const onBackgroundPointerDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (drawMode && e.button === 0) {
      addNoteAt(e, true);
      return;
    }
    marquee.onPointerDown(e);
  };

  const onBackgroundDoubleClick = (e: React.MouseEvent<HTMLDivElement>) => {
    if (drawMode || e.target !== e.currentTarget) return;
    addNoteAt(e, false);
  };

  const onNotePointerDown = (e: ReactPointerEvent<HTMLDivElement>, note: Note) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    const sel = itemSelection.getState();
    const mode = selectModeFromEvent(e);
    const wasSelected = sel.selected.note.has(note.id);
    if (!wasSelected) sel.select("note", [note.id], mode === "remove" ? "replace" : mode);
    else if (mode === "toggle") {
      sel.select("note", [note.id], "remove");
      return;
    }
    const box = e.currentTarget.getBoundingClientRect();
    const zone = noteHitZone(e.clientX - box.left, box.width);
    const selected = itemSelection.getState().selected.note;
    const originals = notes.filter((n) => selected.has(n.id));
    startDrag(
      transport,
      e,
      {
        move: (dx, dy, ev) => {
          const snap = ev.altKey ? null : step;
          const dBeats = dx / vp.pxPerBeat;
          const targetY = pitchToY(note.pitch, keyH, rows) + Math.round(dy / keyH) * keyH;
          const dPitch = yToPitch(targetY, keyH, rows) - note.pitch;
          const edits =
            zone === "body"
              ? moveEdits(originals, note, dBeats, dPitch, snap, tempo)
              : resizeEdits(originals, note, zone, dBeats, snap, tempo);
          return cmd("Note", { type: "Edit", edits });
        },
        end: (moved) => {
          // A plain click on an already-selected note selects just that note.
          if (!moved && wasSelected && mode === "replace") itemSelection.getState().select("note", [note.id], "replace");
        },
      },
      { threshold: 3 },
    );
  };

  const onNoteDoubleClick = (e: React.MouseEvent, id: NoteId) => {
    e.stopPropagation();
    itemSelection.getState().select("note", [id], "remove");
    void send(cmd("Note", { type: "Remove", ids: [id] }));
  };

  // Playable region of the clip on its content axis.
  const regionStart = clip.looping.enabled ? clip.looping.start : clip.offset;
  const regionEnd = contentEnd(clip);
  const xStart = beatsToPx(regionStart, vp);
  const xEnd = beatsToPx(regionEnd, vp);

  return (
    <div
      ref={rootRef}
      className={clsx("eth-pr-grid", drawMode && "eth-pr-grid--draw")}
      style={{ height }}
      data-testid="piano-roll-grid"
      onPointerDown={onBackgroundPointerDown}
      onDoubleClick={onBackgroundDoubleClick}
    >
      <Rows keyH={keyH} rows={rows} scale={scale} highlight={highlight} />
      {lines.map((l) => {
        const x = beatsToPx(l.beats, vp);
        return <div key={l.beats} className={`eth-pr-grid__line eth-pr-grid__line--${l.level}`} style={{ transform: `translateX(${Math.round(x)}px)` }} />;
      })}
      <div className="eth-pr-grid__outside" style={{ left: 0, width: Math.max(0, xStart) }} />
      <div className="eth-pr-grid__outside" style={{ left: Math.max(0, xEnd), right: 0 }} data-testid="piano-roll-clip-end" />
      {visible.map((n) => (
        <NoteView key={n.id} note={n} vp={vp} keyH={keyH} rows={rows} tone={scaleTone(n.pitch, scale, highlight)} onPointerDown={onNotePointerDown} onDoubleClick={onNoteDoubleClick} />
      ))}
      {marquee.rect && (
        <div
          className="eth-pr-grid__marquee"
          style={{
            left: marquee.rect.x0,
            top: marquee.rect.y0,
            width: marquee.rect.x1 - marquee.rect.x0,
            height: marquee.rect.y1 - marquee.rect.y0,
          }}
        />
      )}
      <PlayheadLine view={view} mapping={(song) => songToContent(clip, song)} />
    </div>
  );
}

/** Black-key row shading and octave (C) lines; static for a given key height. */
const Rows = memo(function Rows({ keyH, rows: pitches, scale, highlight }: Pick<NoteGridProps, "keyH" | "rows" | "scale" | "highlight">) {
  const rows = [];
  for (const p of pitches) {
    const black = isBlackKey(p);
    const c = p % 12 === 0;
    const tone = scaleTone(p, scale, highlight);
    rows.push(
      <div
        key={p}
        data-pitch={p}
        data-scale-tone={tone}
        className={clsx("eth-pr-grid__row", black && "eth-pr-grid__row--black", c && "eth-pr-grid__row--c")}
        style={{ top: pitchToY(p, keyH, pitches), height: keyH }}
      />,
    );
  }
  return <>{rows}</>;
});

interface NoteViewProps {
  rows: readonly number[];
  tone: ReturnType<typeof scaleTone>;
  note: Note;
  vp: TimelineViewport;
  keyH: number;
  onPointerDown: (e: ReactPointerEvent<HTMLDivElement>, note: Note) => void;
  onDoubleClick: (e: React.MouseEvent, id: NoteId) => void;
}

function NoteView({ note, vp, keyH, rows, tone, onPointerDown, onDoubleClick }: NoteViewProps) {
  const selected = useIsSelected("note", note.id);
  const r = noteRect(note, vp, keyH, rows);
  return (
    <div
      className={clsx("eth-pr-note", selected && "eth-pr-note--selected", note.muted && "eth-pr-note--muted")}
      data-testid="piano-roll-note"
      data-note-id={note.id}
      data-pitch={note.pitch}
      data-scale-tone={tone}
      style={{
        left: r.x0,
        top: r.y0,
        width: Math.max(2, r.x1 - r.x0),
        height: keyH,
        opacity: (0.45 + 0.55 * note.velocity) * (tone === "out" && !selected ? 0.45 : 1),
      }}
      onPointerDown={(e) => onPointerDown(e, note)}
      onDoubleClick={(e) => onDoubleClick(e, note.id)}
    />
  );
}
