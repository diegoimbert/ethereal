/**
 * The note grid: pitch rows, grid lines, the clip's playable region, notes and the
 * marquee. Draw / move / resize notes; every drag is one undo gesture.
 */

import { memo, useLayoutEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import clsx from "clsx";
import { scaleTone } from "@/domain/scales";
import type { Clip, Command, MusicalScale, Note, NoteId } from "@/generated";
import { openContextMenu, type ContextMenuEntry } from "@/kit";
import { useProjectStore } from "@/state";
import {
  beatsToPx,
  createDoublePress,
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
import { EditorPresence, type EditorCursorMapping } from "@/features/collab/presence";
import { contentEnd, contentToSong, songToContent } from "./clipTime";
import { startDrag, useSend } from "./drag";
import { isBlackKey, noteHitZone, noteRect, pitchToY, rowPitchDelta, yToPitch } from "./geometry";
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
  /** Extra note context-menu items for the clicked selection (appended after Delete). */
  menuItems?: (ids: NoteId[]) => ContextMenuEntry[];
}

const BAR_STEP: GridStep = { kind: "bars", bars: 1 };

export function NoteGrid({ clip, notes, view, vp, widthPx, keyH, rows, scale, highlight, tempo, step, newNoteBeats, drawMode, menuItems }: NoteGridProps) {
  const transport = useTransport();
  const send = useSend();
  const rootRef = useRef<HTMLDivElement>(null);
  const height = rows.length * keyH;

  // Collab: grid px ⇄ content beats and pitch (the key plus how far up it), for peers'
  // cursors on this clip.
  const cursorMapping = useRef<EditorCursorMapping>(null!);
  useLayoutEffect(() => {
    cursorMapping.current = {
      fromScreen: (x, y) => {
        const row = Math.min(rows.length - 1, Math.max(0, Math.floor(y / keyH)));
        const within = Math.min(0.999, Math.max(0, 1 - (y - row * keyH) / keyH));
        return { beats: pxToBeats(x, vp), pitch: rows[row]! + within };
      },
      toScreen: (beats, pitch) => {
        const key = Math.floor(pitch);
        return { x: beatsToPx(beats, vp), y: pitchToY(key, keyH, rows) + (1 - (pitch - key)) * keyH };
      },
    };
  });

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
    // A click on empty space (no drag) also moves the playhead there, snapped to the grid
    // (alt: free), while stopped, like in the arrangement.
    onClick: (p, ev) => {
      if (useProjectStore.getState().transport?.playing) return;
      const content = Math.max(0, snapToGrid(pxToBeats(p.x, vp), ev.altKey ? null : step, tempo, "nearest"));
      transport.send(cmd("Transport", { type: "Locate", position: contentToSong(clip, content) })).catch(() => {});
    },
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
      { initial: add, afterInitial: selectIt, threshold: 3, cursor: "ew-resize" },
    );
  };

  const [isDoublePress] = useState(createDoublePress);

  const onBackgroundPointerDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    // Draw mode: every press inserts. Otherwise the second press of a double-click on empty
    // space inserts, and dragging before releasing sets the new note's length.
    const insert = e.button === 0 && (drawMode || (e.target === e.currentTarget && isDoublePress(e)));
    if (insert) {
      addNoteAt(e, true);
      return;
    }
    marquee.onPointerDown(e);
  };

  const onNotePointerDown = (e: ReactPointerEvent<HTMLDivElement>, note: Note) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    const sel = itemSelection.getState();
    const mode = selectModeFromEvent(e);
    const wasSelected = sel.selected.note.has(note.id);
    // Cmd/ctrl: a click toggles the note (on release), a drag duplicates the selection.
    if (!wasSelected) sel.select("note", [note.id], mode === "remove" || mode === "toggle" ? "add" : mode);
    const box = e.currentTarget.getBoundingClientRect();
    const zone = noteHitZone(e.clientX - box.left, box.width);
    const selected = itemSelection.getState().selected.note;
    const originals = notes.filter((n) => selected.has(n.id));
    /** Copy ids by original id while the drag duplicates (cmd/ctrl held). */
    let copies: Map<NoteId, NoteId> | null = null;
    const back = originals.map((n) => noteEdit(n.id, { start: n.start, pitch: n.pitch }));
    startDrag(
      transport,
      e,
      {
        move: (dx, dy, ev) => {
          const snap = ev.altKey ? null : step;
          const dBeats = dx / vp.pxPerBeat;
          if (zone !== "body") return cmd("Note", { type: "Edit", edits: resizeEdits(originals, note, zone, dBeats, snap, tempo) });
          const edits = moveEdits(originals, note, dBeats, rowPitchDelta(note.pitch, dy, keyH, rows), snap, tempo);
          const wantCopy = ev.metaKey || ev.ctrlKey;
          if (wantCopy && !copies) {
            // Cmd pressed (at the start or mid-drag): the originals go back where they were
            // and copies take their place under the pointer.
            copies = new Map(originals.map((n) => [n.id, newId()]));
            const a = edits.find((x) => x.id === note.id)!;
            return batch("Duplicate Notes", [
              cmd("Note", { type: "Edit", edits: back }),
              cmd("Note", {
                type: "Duplicate",
                copies: [...copies].map(([from, new_id]) => ({ from, new_id })),
                offset: (a.start ?? note.start) - note.start,
                transpose: (a.pitch ?? note.pitch) - note.pitch,
              }),
            ]);
          }
          if (!wantCopy && copies) {
            // Cmd released: drop the copies, the originals follow the pointer again.
            const ids = [...copies.values()];
            copies = null;
            return batch("Move Notes", [cmd("Note", { type: "Remove", ids }), cmd("Note", { type: "Edit", edits })]);
          }
          const ids = copies;
          return cmd("Note", { type: "Edit", edits: ids ? edits.map((x) => ({ ...x, id: ids.get(x.id)! })) : edits });
        },
        end: (moved) => {
          if (copies) {
            const project = useProjectStore.getState().project;
            const created = [...copies.values()].filter((id) => project?.notes[id]);
            if (created.length) itemSelection.getState().select("note", created, "replace");
          } else if (!moved && wasSelected) {
            // A plain click on a selected note selects just it; cmd-click deselects it.
            if (mode === "replace") itemSelection.getState().select("note", [note.id], "replace");
            else if (mode === "toggle") itemSelection.getState().select("note", [note.id], "remove");
          }
        },
      },
      {
        threshold: 3,
        cursor: (m) => (zone !== "body" ? "ew-resize" : m.metaKey || m.ctrlKey ? "copy" : "move"),
      },
    );
  };

  const onNoteDoubleClick = (e: React.MouseEvent, id: NoteId) => {
    e.stopPropagation();
    itemSelection.getState().select("note", [id], "remove");
    void send(cmd("Note", { type: "Remove", ids: [id] }));
  };

  const onNoteContextMenu = (e: React.MouseEvent, id: NoteId) => {
    const sel = itemSelection.getState();
    if (!sel.selected.note.has(id)) sel.select("note", [id], "replace");
    const ids = [...itemSelection.getState().selected.note].filter((n) => notes.some((x) => x.id === n));
    openContextMenu(e, [
      {
        label: ids.length > 1 ? `Delete ${ids.length} Notes` : "Delete Note",
        shortcut: "⌫",
        danger: true,
        onSelect: () => {
          itemSelection.getState().select("note", ids, "remove");
          void send(cmd("Note", { type: "Remove", ids }));
        },
      },
      ...(menuItems ? ["separator" as const, ...menuItems(ids)] : []),
    ]);
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
    >
      <Rows keyH={keyH} rows={rows} scale={scale} highlight={highlight} />
      {lines.map((l) => {
        const x = beatsToPx(l.beats, vp);
        return <div key={l.beats} className={`eth-pr-grid__line eth-pr-grid__line--${l.level}`} style={{ transform: `translateX(${Math.round(x)}px)` }} />;
      })}
      <div className="eth-pr-grid__outside" style={{ left: 0, width: Math.max(0, xStart) }} />
      <div className="eth-pr-grid__outside" style={{ left: Math.max(0, xEnd), right: 0 }} data-testid="piano-roll-clip-end" />
      {visible.map((n) => (
        <NoteView
          key={n.id}
          note={n}
          vp={vp}
          keyH={keyH}
          rows={rows}
          tone={scaleTone(n.pitch, scale, highlight)}
          onPointerDown={onNotePointerDown}
          onDoubleClick={onNoteDoubleClick}
          onContextMenu={onNoteContextMenu}
        />
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
      <EditorPresence clip={clip.id} gridRef={rootRef} mapping={cursorMapping} />
    </div>
  );
}

/** Several commands as one (inside the drag's gesture). */
function batch(label: string, commands: Command[]): Command {
  return cmd("Edit", { type: "Batch", label, commands });
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
  onContextMenu: (e: React.MouseEvent, id: NoteId) => void;
}

function NoteView({ note, vp, keyH, rows, tone, onPointerDown, onDoubleClick, onContextMenu }: NoteViewProps) {
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
        opacity: 0.45 + 0.55 * note.velocity,
      }}
      onPointerDown={(e) => onPointerDown(e, note)}
      onDoubleClick={(e) => onDoubleClick(e, note.id)}
      onContextMenu={(e) => onContextMenu(e, note.id)}
    />
  );
}
