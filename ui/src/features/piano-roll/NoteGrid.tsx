/**
 * The note grid: pitch rows, grid lines, the clip's playable region, notes and the
 * marquee. Draw / move / resize notes; every drag is one undo gesture. The marquee also
 * makes the section (a time range, like the arrangement's time selection; `section.ts`).
 * Alt/Option + drag on a note body changes the velocity of the selection (or that note)
 * instead of moving it; Shift moves notes without snapping.
 */

import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import clsx from "clsx";
import { scaleTone } from "@/domain/scales";
import type { Clip, Command, MusicalScale, Note, NoteId } from "@/generated";
import { Badge, openContextMenu, type ContextMenuEntry } from "@/kit";
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
import { EditorNotes, leaveNoteEntries, withSeparator } from "@/features/collab/social";
import { NotePitchCurves } from "@/features/mpe";
import { contentEnd, contentToSong, songToContent } from "./clipTime";
import { startDrag, useSend } from "./drag";
import { isBlackKey, noteHitZone, noteRect, pitchToY, rowPitchDelta, yToPitch } from "./geometry";
import { midiVelocity, moveEdits, newNote, noteEdit, resizeEdits, velocityDrag, velocityEdits } from "./noteEdits";
import { setPlayStart } from "@/features/time-edits/marker";
import { clearSection, isSectionMarker, placeSectionMarker, sectionOf, usePianoRollSection, type PianoRollSection } from "./section";

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
  /**
   * Snap step (`null` = grid off). Alt bypasses snapping during a drag, except when moving
   * notes: there Alt edits velocity and Shift bypasses snapping.
   */
  step: GridStep | null;
  /** Length of a new note (the grid step, or a 1/16 when the grid is off). */
  newNoteBeats: number;
  drawMode: boolean;
  /** Extra note context-menu items for the clicked selection (appended after Delete). */
  menuItems?: (ids: NoteId[]) => ContextMenuEntry[];
  /** The section (time range) of this clip, drawn over the rows. */
  section?: PianoRollSection | null;
}

const BAR_STEP: GridStep = { kind: "bars", bars: 1 };

export function NoteGrid({ clip, notes, view, vp, widthPx, keyH, rows, scale, highlight, tempo, step, newNoteBeats, drawMode, menuItems, section = null }: NoteGridProps) {
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

  /** Alt held at the marquee's press: its section is not snapped. */
  const marqueeAlt = useRef(false);
  const marquee = useMarquee({
    kind: "note",
    hitTest: (rect) => marqueeHits(rect, notes.map((n) => ({ id: n.id, rect: noteRect(n, vp, keyH, rows) }))),
    // The dragged time range becomes the section (snapped; alt: free).
    onEnd: (rect) => {
      const at = (px: number) => Math.max(0, snapToGrid(pxToBeats(px, vp), marqueeAlt.current ? null : step, tempo, "nearest"));
      usePianoRollSection.getState().setSection(sectionOf(clip.id, at(rect.x0), at(rect.x1)));
    },
    // A click on empty space (no drag) places the insert marker there, snapped to the grid
    // (alt: free). It never moves a playing playhead; while stopped, Play starts from it.
    onClick: (p, ev) => {
      const content = Math.max(0, snapToGrid(pxToBeats(p.x, vp), ev.altKey ? null : step, tempo, "nearest"));
      placeSectionMarker(clip.id, content);
      setPlayStart(transport, contentToSong(clip, content));
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
  /** Alt held over the grid: a press on a note body edits velocity (ns-resize cursor). */
  const altHeld = useAltHeld();
  /** The note under an Alt-drag (its velocity is shown in a badge while dragging). */
  const [velocityNote, setVelocityNote] = useState<NoteId | null>(null);

  /**
   * Alt-drag on a note body: vertical movement changes the velocity of `originals` (the
   * selection, or just the pressed note) by the same amount; one undo gesture, no time or
   * pitch change. A click without a drag changes nothing.
   */
  const startVelocityDrag = (e: ReactPointerEvent, note: Note, originals: Note[], onClick: () => void) => {
    const dv = velocityDrag();
    setVelocityNote(note.id);
    startDrag(
      transport,
      e,
      {
        move: (_dx, dy, ev) => cmd("Note", { type: "Edit", edits: velocityEdits(originals, dv(dy, ev.shiftKey)) }),
        end: (moved) => {
          setVelocityNote(null);
          if (!moved) onClick();
        },
      },
      { threshold: 3, cursor: "ns-resize" },
    );
  };

  const onBackgroundPointerDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    // Draw mode: every press inserts. Otherwise the second press of a double-click on empty
    // space inserts, and dragging before releasing sets the new note's length.
    const insert = e.button === 0 && (drawMode || (e.target === e.currentTarget && isDoublePress(e)));
    if (insert) {
      addNoteAt(e, true);
      return;
    }
    marqueeAlt.current = e.altKey;
    marquee.onPointerDown(e);
  };

  const onNotePointerDown = (e: ReactPointerEvent<HTMLDivElement>, note: Note) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    // Pressing a note leaves section mode: edits act on the selected notes again.
    clearSection();
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
    /** A click (no drag) on a selected note: plain selects just it; cmd deselects it. */
    const clickSelected = () => {
      if (!wasSelected) return;
      if (mode === "replace") itemSelection.getState().select("note", [note.id], "replace");
      else if (mode === "toggle") itemSelection.getState().select("note", [note.id], "remove");
    };
    if (zone === "body" && e.altKey) {
      startVelocityDrag(e, note, originals, clickSelected);
      return;
    }
    const back = originals.map((n) => noteEdit(n.id, { start: n.start, pitch: n.pitch }));
    startDrag(
      transport,
      e,
      {
        move: (dx, dy, ev) => {
          const dBeats = dx / vp.pxPerBeat;
          // Resize: alt bypasses snapping. Move: shift does (alt at the press edits velocity
          // instead; alt pressed mid-move still frees it).
          if (zone !== "body") return cmd("Note", { type: "Edit", edits: resizeEdits(originals, note, zone, dBeats, ev.altKey ? null : step, tempo) });
          const snap = ev.shiftKey || ev.altKey ? null : step;
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
          } else if (!moved) {
            clickSelected();
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
      // collab-social: pin a note here (hidden while "Hide users and notes" is on).
      ...withSeparator(leaveNoteEntries({ kind: "editor", clip: clip.id }, e)),
    ]);
  };

  /** Empty grid: "Leave a note" (collab-social); nothing else is offered there. */
  const onBackgroundContextMenu = (e: React.MouseEvent) => openContextMenu(e, leaveNoteEntries({ kind: "editor", clip: clip.id }, e));

  // Playable region of the clip on its content axis.
  const regionStart = clip.looping.enabled ? clip.looping.start : clip.offset;
  const regionEnd = contentEnd(clip);
  const xStart = beatsToPx(regionStart, vp);
  const xEnd = beatsToPx(regionEnd, vp);

  return (
    <div
      ref={rootRef}
      className={clsx("eth-pr-grid", drawMode && "eth-pr-grid--draw", altHeld && !drawMode && "eth-pr-grid--alt")}
      style={{ height }}
      data-testid="piano-roll-grid"
      onPointerDown={onBackgroundPointerDown}
      onContextMenu={onBackgroundContextMenu}
    >
      <Rows keyH={keyH} rows={rows} scale={scale} highlight={highlight} />
      {lines.map((l) => {
        const x = beatsToPx(l.beats, vp);
        return <div key={l.beats} className={`eth-pr-grid__line eth-pr-grid__line--${l.level}`} style={{ transform: `translateX(${Math.round(x)}px)` }} />;
      })}
      <div className="eth-pr-grid__outside" style={{ left: 0, width: Math.max(0, xStart) }} />
      <div className="eth-pr-grid__outside" style={{ left: Math.max(0, xEnd), right: 0 }} data-testid="piano-roll-clip-end" />
      {section && isSectionMarker(section) ? (
        <div className="eth-pr-grid__marker" data-testid="piano-roll-marker" data-beats={section.start} style={{ left: beatsToPx(section.start, vp) }} />
      ) : section && (
        <div
          className="eth-pr-grid__section"
          data-testid="piano-roll-section"
          style={{ left: beatsToPx(section.start, vp), width: (section.end - section.start) * vp.pxPerBeat }}
        />
      )}
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
      {velocityNote && <VelocityBadge note={notes.find((n) => n.id === velocityNote)} vp={vp} keyH={keyH} rows={rows} />}
      <NotePitchCurves notes={visible} vp={vp} keyH={keyH} rowY={(p) => pitchToY(p, keyH, rows)} widthPx={widthPx} height={height} />
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
      <EditorNotes clip={clip.id} gridRef={rootRef} mapping={cursorMapping} layoutKey={`${vp.pxPerBeat}:${vp.scrollBeats}:${keyH}:${rows.length}:${rows[0] ?? 0}`} />
    </div>
  );
}

/** Whether Alt/Option is held (for the velocity-drag cursor over notes). */
function useAltHeld(): boolean {
  const [held, setHeld] = useState(false);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => setHeld(e.altKey);
    const onMove = (e: PointerEvent) => setHeld(e.altKey);
    const off = () => setHeld(false);
    window.addEventListener("keydown", onKey);
    window.addEventListener("keyup", onKey);
    window.addEventListener("pointermove", onMove);
    window.addEventListener("blur", off);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("keyup", onKey);
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("blur", off);
    };
  }, []);
  return held;
}

/** The velocity (MIDI 1..127) of the note under an Alt-drag, just above it. */
function VelocityBadge({ note, vp, keyH, rows }: { note: Note | undefined; vp: TimelineViewport; keyH: number; rows: readonly number[] }) {
  if (!note) return null;
  const r = noteRect(note, vp, keyH, rows);
  return (
    <div
      // Below the note when it sits on the top rows (the badge would be clipped above).
      className={clsx("eth-pr-grid__velocity", r.y0 < 2 * keyH && "eth-pr-grid__velocity--below")}
      data-testid="piano-roll-velocity-badge"
      style={{ left: r.x0, top: r.y0 < 2 * keyH ? r.y0 + keyH : r.y0 }}
      aria-live="polite"
    >
      <Badge tone="accent">Velocity {midiVelocity(note.velocity)}</Badge>
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
