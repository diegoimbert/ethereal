/**
 * The note grid: pitch rows, grid lines, the clip's playable region, notes and the
 * marquee. Draw / move / resize notes; every drag is one undo gesture. A Shift-drag makes
 * the section instead (a time range, like the arrangement's time selection; `section.ts`).
 * Alt/Option + drag on a note body changes the velocity of the selection (or that note)
 * instead of moving it; Alt pressed once a move has started turns its snapping off (Alt is
 * the snap bypass of every drag).
 *
 * With several clips (multi-clip editing, `multiClip.ts`) the axis is the song timeline:
 * each clip is a band, the space outside every clip is shaded, notes take their clip's color
 * and the focused clip's notes are drawn brighter; a press on another clip's note focuses it.
 *
 * Preview (`notePreview.ts`, toolbar toggle): pressing a note, or drawing one, plays it on
 * its clip's track until release; a vertical drag re-plays it at each new pitch.
 *
 * Drawing: the rows, grid lines, loop ghosts and notes are drawn on two canvases (WebGL2,
 * `noteCanvas.ts` / `noteLayout.ts`), not as DOM elements (thousands of notes made zooming
 * and scrolling lag). The canvases cover the visible rows only (plus a margin) and follow the
 * vertical scroll; the few single elements (shaded outside, clip starts, section, marker,
 * marquee, playhead, collab) stay DOM, between the rows canvas and the notes canvas as they
 * were stacked before. Pointer events land on the grid and are hit-tested against the notes
 * (`hitNote`); the cursor follows what is under the pointer (`gridCursor`).
 */

import { surfaceMenuEntries } from "@/ext/surfaces";
import { useContributions, type EditorCursorMapping } from "@/ext";
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent, type RefObject } from "react";
import clsx from "clsx";
import type { ClipId, Command, MusicalScale, Note, NoteId, TextKey } from "@/generated";
import { t, useT } from "@/i18n";
import { Badge, openContextMenu, useScopedDrag, type ContextMenuEntry } from "@/kit";
import { runAction, shortcutLabel } from "@/features/keymap";
import { useEditorStore, useProjectStore } from "@/state";
import {
  beatsToPx,
  dragBeats,
  createDoublePress,
  extendOrRetract,
  gridLines,
  itemSelection,
  marqueeHits,
  PlayheadLine,
  pxToBeats,
  selectModeFromEvent,
  snapToGrid,
  useMarquee,
  visibleRange,
  type GridStep,
  type Rect,
  type TempoMap,
  type TimelineViewport,
  type TimelineViewStore,
} from "@/timeline";
import { border, opacity as opacityTokens, radius, readToken, size, useGlow, useTheme, useThemeColor } from "@/theme";
import { parseColor, type Rgba } from "@/features/arrangement/gl/palette";
import { cmd, newId, useTransport } from "@/transport";
import { NotePitchCurves } from "@/features/mpe";
import { contentEnd } from "./clipTime";
import { startDrag, useSend } from "./drag";
import { noteRect, pitchToY, rowPitchDelta, yToPitch } from "./geometry";
import { midiVelocity, moveEdits, newNote, noteEdit, resizeEdits, velocityDrag, velocityEdits } from "./noteEdits";
import { newNoteVelocity, touchNote } from "./lastVelocity";
import { InstanceList, RectRenderer } from "./noteCanvas";
import {
  buildNotesLayer,
  buildRowsLayer,
  gridCursor,
  gridPalette,
  hitNote,
  registerScene,
  tagged,
  type GridPalette,
  type LayerFrame,
  type NoteHit,
  type SceneTag,
} from "./noteLayout";
import { setPlayStart } from "@/features/time-edits/marker";
import {
  clearSection,
  isSectionMarker,
  notesOver,
  placeSectionMarker,
  sectionHasPitch,
  sectionOf,
  sectionStep,
  usePianoRollSection,
  type PianoRollSection,
} from "./section";
import { outsideBands, type GhostNote } from "./multiClip";
import { playPreview, stopPreview } from "./notePreview";
import type { RollSpace } from "./rollSpace";

export interface NoteGridProps {
  /** The edited clip(s) and the time axis (`rollSpace.ts`). */
  space: RollSpace;
  notes: ReadonlyArray<Note>;
  /** Loop repetitions of the notes (multi-clip editing; drawn, not interactive). */
  ghosts?: ReadonlyArray<GhostNote>;
  /** Note color by clip (multi-clip editing: the clip's or its track's color). */
  clipColors?: ReadonlyMap<ClipId, string>;
  view: TimelineViewStore;
  vp: TimelineViewport;
  widthPx: number;
  keyH: number;
  rows: readonly number[];
  scale: MusicalScale;
  highlight: boolean;
  tempo: TempoMap;
  /**
   * Snap step (`null` = grid off). Alt bypasses snapping during every drag. On a note body
   * Alt held at the press edits velocity instead, so a move is freed by Alt pressed mid-move.
   */
  step: GridStep | null;
  /** Length of a new note (the grid step, or a 1/16 when the grid is off). */
  newNoteBeats: number;
  drawMode: boolean;
  /** Extra note context-menu items for the clicked selection (appended after Delete). */
  menuItems?: (ids: NoteId[]) => ContextMenuEntry[];
  /** The section (time range) of this clip, drawn over the rows. */
  section?: PianoRollSection | null;
  /**
   * The vertically scrolling element around the grid: the canvases only cover its visible
   * part. Without it they cover the whole grid.
   */
  scrollRef?: RefObject<HTMLElement | null>;
}

const BAR_STEP: GridStep = { kind: "bars", bars: 1 };
/** Rows drawn above and below the visible part (px), so a fast scroll shows no gap. */
const OVERSCAN_PX = 160;
/** Drawing width when the view's width is unknown (no layout, e.g. tests). */
const FALLBACK_WIDTH = 2000;

export function NoteGrid({
  space,
  notes,
  ghosts,
  clipColors,
  view,
  vp,
  widthPx,
  keyH,
  rows,
  scale,
  highlight,
  tempo,
  step,
  newNoteBeats,
  drawMode,
  menuItems,
  section = null,
  scrollRef,
}: NoteGridProps) {
  const clip = space.focus;
  const transport = useTransport();
  const send = useSend();
  const windowDrag = useScopedDrag(startDrag);
  const rootRef = useRef<HTMLDivElement>(null);
  const height = rows.length * keyH;
  // The editor closing (or switching clips) never leaves a preview note sounding.
  useEffect(() => stopPreview, []);
  /** Preview `pitch` on the track of `clipId` (the note's clip; multi-clip editing). */
  const preview = (clipId: ClipId, pitch: number, velocity: number) => {
    const track = (space.clips.find((c) => c.id === clipId) ?? clip).track;
    playPreview(transport, track, pitch, velocity);
  };

  // Grid px ⇄ content beats (of the focused clip) and pitch (the key plus how far up it), for
  // the extensions' layers (`@/ext` "pianoRoll.layer": e.g. other people's cursors).
  const layers = useContributions("pianoRoll.layer");
  const cursorMapping = useRef<EditorCursorMapping>(null!);
  useLayoutEffect(() => {
    const shift = space.focusShift;
    cursorMapping.current = {
      fromScreen: (x, y) => {
        const row = Math.min(rows.length - 1, Math.max(0, Math.floor(y / keyH)));
        const within = Math.min(0.999, Math.max(0, 1 - (y - row * keyH) / keyH));
        return { beats: pxToBeats(x, vp) - shift, pitch: rows[row]! + within };
      },
      toScreen: (beats, pitch) => {
        const key = Math.floor(pitch);
        return { x: beatsToPx(beats + shift, vp), y: pitchToY(key, keyH, rows) + (1 - (pitch - key)) * keyH };
      },
    };
  });

  const range = useMemo(() => visibleRange(vp, widthPx > 0 ? widthPx : FALLBACK_WIDTH), [vp, widthPx]);
  const lines = useMemo(() => gridLines(tempo, range, step ?? BAR_STEP), [tempo, range, step]);
  const visible = useMemo(
    () => (widthPx > 0 ? notes.filter((n) => n.start <= range.end && n.start + n.duration >= range.start) : notes),
    [notes, range, widthPx],
  );
  const geometry = { vp, keyH, rows };

  /** Alt held at the marquee's press: its section is not snapped. */
  const marqueeAlt = useRef(false);
  /** The section from a drag rectangle (snapped; alt: free). */
  const sectionFromRect = (rect: Rect): PianoRollSection | null => {
    const at = (px: number) => Math.max(space.floor, snapToGrid(pxToBeats(px, vp), marqueeAlt.current ? null : sectionStep(step), tempo, "nearest"));
    // The rows it was drawn over (like the arrangement's section covers its tracks).
    const a = yToPitch(rect.y0, keyH, rows);
    const b = yToPitch(Math.max(rect.y0, rect.y1 - 1e-3), keyH, rows);
    return sectionOf(space.key, at(rect.x0), at(rect.x1), { low: Math.min(a, b), high: Math.max(a, b) }, space.floor);
  };
  // A plain drag selects notes; a Shift-drag makes a section AND selects the notes under the
  // rectangle (notes and a section at once, like the arrangement's section covering its clips),
  // both live while dragging. Shift can be pressed or released mid-drag to switch.
  const hitNotes = (rect: Rect) =>
    marqueeHits(
      rect,
      notes.map((n) => ({ id: n.id, rect: noteRect(n, vp, keyH, rows) })),
    );
  const marquee = useMarquee({
    kind: "note",
    hitTest: hitNotes,
    section: {
      onPreview: (rect) => {
        if (!rect) return clearSection();
        itemSelection.getState().select("note", hitNotes(rect), "replace");
        usePianoRollSection.getState().setSection(sectionFromRect(rect));
      },
      onEnd: (rect) => {
        itemSelection.getState().select("note", hitNotes(rect), "replace");
        const sec = sectionFromRect(rect);
        usePianoRollSection.getState().setSection(sec);
        // The section start is the selection cursor (the playhead follows while stopped).
        if (sec) setPlayStart(transport, space.toSong(sec.start));
      },
      // A Shift-click extends the section (or the insert marker) to the click when outside it,
      // retracts its nearer edge when inside (`extendOrRetract`), and spans the clicked row
      // when it covers some rows; the notes under it are selected, like after a Shift-drag.
      onShiftClick: (p, ev) => {
        const prev = usePianoRollSection.getState().section;
        if (!prev || prev.clip !== space.key) return false;
        const snap = (b: number) => snapToGrid(b, ev.altKey ? null : sectionStep(step), tempo, "nearest");
        const next = extendOrRetract(prev, pxToBeats(p.x, vp), snap);
        const pitch = yToPitch(p.y, keyH, rows);
        const pitches = prev.pitches && { low: Math.min(prev.pitches.low, pitch), high: Math.max(prev.pitches.high, pitch) };
        const sec = sectionOf(space.key, next.start, next.end, pitches, space.floor);
        if (!sec) {
          placeSectionMarker(space.key, next.start, space.floor);
          setPlayStart(transport, space.toSong(next.start));
          return true;
        }
        usePianoRollSection.getState().setSection(sec);
        const under = notesOver(notes, sec.start, sec.end).filter((n) => sectionHasPitch(sec, n.pitch));
        itemSelection.getState().select(
          "note",
          under.map((n) => n.id),
          "replace",
        );
        setPlayStart(transport, space.toSong(sec.start));
        return true;
      },
    },
    // A click on empty space (no drag) places the insert marker there, snapped to the grid
    // (alt: free). It never moves a playing playhead; while stopped, Play starts from it.
    onClick: (p, ev) => {
      const content = Math.max(space.floor, snapToGrid(pxToBeats(p.x, vp), ev.altKey ? null : step, tempo, "nearest"));
      placeSectionMarker(space.key, content, space.floor);
      setPlayStart(transport, space.toSong(content));
    },
  });

  const local = (e: { clientX: number; clientY: number }) => {
    const box = rootRef.current!.getBoundingClientRect();
    return { x: e.clientX - box.left, y: e.clientY - box.top };
  };
  /** The note under a pointer event (grid hit test), if any. */
  const noteAt = (e: { clientX: number; clientY: number }): NoteHit | null => {
    const { x, y } = local(e);
    return hitNote(notes, itemSelection.getState().selected.note, x, y, geometry);
  };
  // The mobile UI's touch layer (#522) asks what a finger is on: a note (selected or not).
  useEffect(() => {
    const el = rootRef.current as (HTMLElement & { ethNoteAt?: (x: number, y: number) => { selected: boolean } | null }) | null;
    if (!el) return;
    el.ethNoteAt = (x, y) => {
      const hit = noteAt({ clientX: x, clientY: y });
      return hit ? { selected: itemSelection.getState().selected.note.has(hit.note.id) } : null;
    };
    return () => {
      delete el.ethNoteAt;
    };
  });

  /**
   * Add a note at the pointer; with `drag`, the drag extends it (one gesture). With several
   * clips the note goes into the clip under the pointer (that clip gets the focus); false when
   * there is none there (nothing is drawn outside a clip).
   */
  const addNoteAt = (e: ReactPointerEvent | React.MouseEvent, drag: boolean): boolean => {
    const { x, y } = local(e);
    const snap = e.altKey ? null : step;
    const start = snapToGrid(pxToBeats(x, vp), snap, tempo, "floor");
    const target = space.place(start);
    if (!target) return false;
    if (target.clip.id !== clip.id) useEditorStore.getState().focusClip(target.clip.id);
    // `spec` in editor beats (the drag's length math); the command in the clip's content.
    // Its velocity is the last touched note's (`lastVelocity.ts`).
    const velocity = newNoteVelocity(useProjectStore.getState().project?.notes);
    const spec = newNote(newId(), yToPitch(y, keyH, rows), start, newNoteBeats, velocity, space.floor);
    touchNote(spec);
    const add = cmd("Note", { type: "Add", clip: target.clip.id, notes: [{ ...spec, start: target.content }] });
    // Select once the note exists (the selection is pruned against the project).
    const selectIt = () => itemSelection.getState().select("note", [spec.id], "replace");
    preview(target.clip.id, spec.pitch, spec.velocity);
    if (!drag) {
      void send(add).then((ok) => ok && selectIt());
      return true;
    }
    windowDrag(
      transport,
      e,
      {
        // The note extends to cover the grid cell under the pointer (through the view of the
        // moment: it may scroll or zoom mid-drag).
        move: (dx, _dy, ev) => {
          const pointer = pxToBeats(x + dx, view.getState());
          const end = snapToGrid(pointer, ev.altKey ? null : step, tempo, "ceil");
          const duration = Math.max(spec.duration, end - spec.start);
          return cmd("Note", { type: "Edit", edits: [noteEdit(spec.id, { duration })] });
        },
      },
      { initial: add, afterInitial: selectIt, threshold: 3, cursor: "ew-resize" },
    );
    return true;
  };

  const [isDoublePress] = useState(createDoublePress);
  /** Alt held over the grid: a press on a note body edits velocity (ns-resize cursor). */
  const altHeld = useAltHeld();
  /** The note under an Alt-drag (its velocity is shown in a badge while dragging). */
  const [velocityNote, setVelocityNote] = useState<NoteId | null>(null);
  /** The note the last press hit (a double-click deletes it only if both presses hit it). */
  const pressedNote = useRef<NoteId | null>(null);

  /**
   * Alt-drag on a note body: vertical movement changes the velocity of `originals` (the
   * selection, or just the pressed note) by the same amount; one undo gesture, no time or
   * pitch change. A click without a drag changes nothing.
   */
  const startVelocityDrag = (e: ReactPointerEvent, note: Note, originals: Note[], onClick: () => void) => {
    const dv = velocityDrag();
    setVelocityNote(note.id);
    windowDrag(
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
    if (insert && addNoteAt(e, true)) return;
    marqueeAlt.current = e.altKey;
    // The old section goes (a click places the marker, a drag makes a new section): it is
    // not widened to the notes the drag selects on the way. With Shift it stays until the
    // drag replaces it: a Shift-click extends or retracts it.
    if (!e.shiftKey) clearSection();
    marquee.onPointerDown(e);
  };

  const onNotePointerDown = (e: ReactPointerEvent<HTMLDivElement>, { note, zone }: NoteHit) => {
    // Pressing a note leaves section mode: edits act on the selected notes again.
    clearSection();
    // Multi-clip editing: pressing another clip's note focuses that clip.
    if (note.clip !== clip.id && space.clips.some((c) => c.id === note.clip)) useEditorStore.getState().focusClip(note.clip);
    const sel = itemSelection.getState();
    const mode = selectModeFromEvent(e);
    const wasSelected = sel.selected.note.has(note.id);
    // Cmd/ctrl: a click toggles the note (on release), a drag duplicates the selection.
    if (!wasSelected) sel.select("note", [note.id], mode === "remove" || mode === "toggle" ? "add" : mode);
    const selected = itemSelection.getState().selected.note;
    const originals = notes.filter((n) => selected.has(n.id));
    // Every press on a note auditions it (plain, shift / cmd selection clicks and drags),
    // and new notes take its velocity (`lastVelocity.ts`).
    preview(note.clip, note.pitch, note.velocity);
    touchNote(note);
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
    // Beats from the press to the pointer through the view of each moment (scroll/zoom mid-drag).
    const pressX = local(e).x;
    const beatsMoved = dragBeats(view.getState, pressX);
    windowDrag(
      transport,
      e,
      {
        move: (dx, dy, ev) => {
          const dBeats = beatsMoved(pressX + dx);
          // Alt bypasses snapping (on a move, alt at the press edits velocity instead, so
          // there it is alt pressed mid-move). Shift does nothing here.
          if (zone !== "body")
            return cmd("Note", { type: "Edit", edits: resizeEdits(originals, note, zone, dBeats, ev.altKey ? null : step, tempo, space.floor) });
          const snap = ev.altKey ? null : step;
          const edits = moveEdits(originals, note, dBeats, rowPitchDelta(note.pitch, dy, keyH, rows), snap, tempo, space.limits);
          // A vertical move re-auditions the pressed note at its new pitch (Ableton).
          preview(note.clip, edits.find((x) => x.id === note.id)?.pitch ?? note.pitch, note.velocity);
          const wantCopy = ev.metaKey || ev.ctrlKey;
          if (wantCopy && !copies) {
            // Cmd pressed (at the start or mid-drag): the originals go back where they were
            // and copies take their place under the pointer.
            copies = new Map(originals.map((n) => [n.id, newId()]));
            const a = edits.find((x) => x.id === note.id)!;
            return batch("duplicate notes", { id: "undo.note.duplicate_selection", args: {} }, [
              // i18n-ignore
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
            return batch("move notes", { id: "undo.note.move_selection", args: {} }, [
              cmd("Note", { type: "Remove", ids }),
              cmd("Note", { type: "Edit", edits }),
            ]);
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

  /** Presses on the grid itself hit-test the notes (overlays such as collab pins do not). */
  const onPointerDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    const hit = e.target === e.currentTarget ? noteAt(e) : null;
    if (e.button === 0) pressedNote.current = hit?.note.id ?? null;
    if (hit && e.button === 0) onNotePointerDown(e, hit);
    else onBackgroundPointerDown(e);
  };

  /** Double-click on a note (both presses on it): delete it. */
  const onDoubleClick = (e: React.MouseEvent) => {
    if (e.target !== e.currentTarget) return;
    const hit = noteAt(e);
    if (!hit || hit.note.id !== pressedNote.current) return;
    const id = hit.note.id;
    itemSelection.getState().select("note", [id], "remove");
    void send(cmd("Note", { type: "Remove", ids: [id] }));
  };

  /**
   * Paste, enabled once notes were copied: the piano roll's own ⌘V (`runAction`, in this
   * piano roll), at the selection cursor like the key.
   */
  const pasteEntry = (grid: Element): ContextMenuEntry => ({
    label: t("piano_roll.menu.paste"),
    shortcut: shortcutLabel("edit.paste"),
    disabled: !usePianoRollSection.getState().clipboard,
    reason: t("piano_roll.menu.paste_reason"),
    onSelect: () => void runAction("edit.paste", grid),
  });

  const onNoteContextMenu = (e: React.MouseEvent, id: NoteId) => {
    const sel = itemSelection.getState();
    if (!sel.selected.note.has(id)) sel.select("note", [id], "replace");
    const ids = [...itemSelection.getState().selected.note].filter((n) => notes.some((x) => x.id === n));
    const grid = e.currentTarget;
    // #575: the clipboard set of the clip and automation menus, run by the piano roll's own
    // key handlers (so a section and multi-clip editing behave as with the keys).
    const edit = (label: string, action: string): ContextMenuEntry => ({
      label,
      shortcut: shortcutLabel(action),
      onSelect: () => void runAction(action, grid),
    });
    openContextMenu(e, [
      edit(t("piano_roll.menu.cut"), "edit.cut"),
      edit(t("piano_roll.menu.copy"), "edit.copy"),
      pasteEntry(grid),
      edit(t("piano_roll.menu.duplicate"), "edit.duplicate"),
      "separator",
      {
        label: t("piano_roll.menu.delete_notes", { count: ids.length }),
        shortcut: shortcutLabel("edit.delete"),
        danger: true,
        onSelect: () => {
          itemSelection.getState().select("note", ids, "remove");
          void send(cmd("Note", { type: "Remove", ids }));
        },
      },
      ...(menuItems ? ["separator" as const, ...menuItems(ids)] : []),
      ...surfaceMenuEntries({ kind: "editor", clip: clip.id }, e),
    ]);
  };

  /** A note: its menu. Empty grid: paste (#575), and the extensions' entries. */
  const onContextMenu = (e: React.MouseEvent) => {
    const hit = e.target === e.currentTarget ? noteAt(e) : null;
    if (hit) onNoteContextMenu(e, hit.note.id);
    else openContextMenu(e, [pasteEntry(e.currentTarget), ...surfaceMenuEntries({ kind: "editor", clip: clip.id }, e)]);
  };

  // ---- Cursor: what is under the pointer (the notes are not elements) ----
  const pointer = useRef<{ clientX: number; clientY: number } | null>(null);
  const updateCursorRef = useRef(() => {});
  useLayoutEffect(() => {
    updateCursorRef.current = () => {
      const el = rootRef.current;
      const p = pointer.current;
      if (!el) return;
      const cursor = p ? gridCursor(noteAt(p), altHeld, drawMode) : "";
      if (el.style.cursor !== cursor) el.style.cursor = cursor;
    };
  });
  // Alt pressed or released over a note switches move ⇄ velocity at once.
  useEffect(() => updateCursorRef.current(), [altHeld, drawMode]);
  const onPointerMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    pointer.current = { clientX: e.clientX, clientY: e.clientY };
    updateCursorRef.current();
  };
  const onPointerLeave = () => {
    pointer.current = null;
  };

  // ---- Drawing ----
  const palette = useGridPalette();
  const colors = useMemo(() => {
    if (!clipColors) return undefined;
    const out = new Map<ClipId, Rgba>();
    for (const [id, css] of clipColors) out.set(id, parseColor(css));
    return out;
  }, [clipColors]);
  const rowsCanvasRef = useRef<HTMLCanvasElement>(null);
  const notesCanvasRef = useRef<HTMLCanvasElement>(null);
  const renderers = useRef<{ rows: RectRenderer; notes: RectRenderer } | null>(null);
  const [lists] = useState(() => ({ rows: new InstanceList(), notes: new InstanceList() }));

  /**
   * The scroller's scroll position and height, cached from its scroll / resize events (reading
   * them while drawing right after a render would force a synchronous layout).
   */
  const scrolled = useRef<{ top: number; viewH: number } | null>(null);
  /** The canvases' span of the grid: the scroller's visible rows plus a margin. */
  const frameNow = (): LayerFrame => {
    const scroller = scrollRef?.current;
    if (scroller && !scrolled.current) scrolled.current = { top: scroller.scrollTop, viewH: scroller.clientHeight };
    const viewH = scrolled.current?.viewH || height;
    const h = Math.min(height, viewH + 2 * OVERSCAN_PX);
    const top = Math.max(0, Math.min((scrolled.current?.top ?? 0) - OVERSCAN_PX, height - h));
    return { top, height: h, width: widthPx > 0 ? widthPx : FALLBACK_WIDTH };
  };
  const build = (frame: LayerFrame, out: { rows: InstanceList; notes: InstanceList }, tags?: { rows: SceneTag[]; notes: SceneTag[] }) => {
    out.rows.clear();
    out.notes.clear();
    buildRowsLayer({ ...geometry, ...frame, scale, highlight, lines }, palette, out.rows, tags?.rows);
    buildNotesLayer(
      {
        ...geometry,
        ...frame,
        notes,
        ghosts,
        selected: itemSelection.getState().selected.note,
        multi: space.multi,
        focus: clip.id,
        clipColors: colors,
        scale,
        highlight,
      },
      palette,
      out.notes,
      tags?.notes,
    );
  };
  /** Draws with the latest render's state (a ref: scroll and selection redraw without one). */
  const drawRef = useRef<() => void>(() => {});
  useLayoutEffect(() => {
    drawRef.current = () => {
      const rowsCanvas = rowsCanvasRef.current;
      const notesCanvas = notesCanvasRef.current;
      if (!rowsCanvas || !notesCanvas) return;
      renderers.current ??= { rows: new RectRenderer(rowsCanvas), notes: new RectRenderer(notesCanvas) };
      const frame = frameNow();
      for (const c of [rowsCanvas, notesCanvas]) {
        const s = c.style;
        const w = `${frame.width}px`;
        const h = `${frame.height}px`;
        const t = `translateY(${frame.top}px)`;
        if (s.width !== w) s.width = w;
        if (s.height !== h) s.height = h;
        if (s.transform !== t) s.transform = t;
      }
      build(frame, lists);
      renderers.current.rows.draw(lists.rows, frame.width, frame.height);
      renderers.current.notes.draw(lists.notes, frame.width, frame.height);
    };
  });
  const frameId = useRef(0);
  // Every render (zoom, notes, rows, theme…) redraws at once, in step with the DOM overlays.
  useLayoutEffect(() => {
    cancelAnimationFrame(frameId.current);
    frameId.current = 0;
    drawRef.current();
  });
  // Scroll, resize and selection changes redraw once per frame, without a render.
  useEffect(() => {
    const schedule = () => {
      if (frameId.current) return;
      frameId.current = requestAnimationFrame(() => {
        frameId.current = 0;
        drawRef.current();
      });
    };
    const scroller = scrollRef?.current;
    const measure = () => {
      if (!scroller) return;
      scrolled.current = { top: scroller.scrollTop, viewH: scroller.clientHeight };
      schedule();
    };
    scroller?.addEventListener("scroll", measure, { passive: true });
    const ro = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    if (scroller) ro?.observe(scroller);
    let selected = itemSelection.getState().selected.note;
    const unsubscribe = itemSelection.subscribe((s) => {
      if (s.selected.note === selected) return;
      selected = s.selected.note;
      schedule();
    });
    return () => {
      scroller?.removeEventListener("scroll", measure);
      ro?.disconnect();
      unsubscribe();
      cancelAnimationFrame(frameId.current);
      frameId.current = 0;
    };
  }, [scrollRef]);
  useEffect(
    () => () => {
      renderers.current?.rows.dispose();
      renderers.current?.notes.dispose();
      renderers.current = null;
    },
    [],
  );
  // Tests and debugging: what the canvases draw (`gridScene`).
  const buildRef = useRef(build);
  useLayoutEffect(() => {
    buildRef.current = build;
  });
  useLayoutEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    return registerScene(el, () => {
      const out = { rows: new InstanceList(), notes: new InstanceList() };
      const tags = { rows: [] as SceneTag[], notes: [] as SceneTag[] };
      // The whole grid (no vertical culling), at the current width.
      buildRef.current({ top: 0, height, width: widthPx > 0 ? widthPx : FALLBACK_WIDTH }, out, tags);
      return { rows: tagged(out.rows, tags.rows), notes: tagged(out.notes, tags.notes) };
    });
  }, [height, widthPx]);

  // Shaded areas: outside the clip's playable region on its content axis; with several
  // clips, outside every clip's band on the song axis.
  const outside = useMemo(() => {
    if (space.multi) return outsideBands(space.bands, Math.min(0, range.start), Math.max(range.end, ...space.bands.map((b) => b.end)) + 1);
    const regionStart = clip.looping.enabled ? clip.looping.start : clip.offset;
    return [
      { start: -Infinity, end: regionStart },
      { start: contentEnd(clip), end: Infinity },
    ];
  }, [space, clip, range]);

  return (
    <div
      ref={rootRef}
      className={clsx("eth-pr-grid", drawMode && "eth-pr-grid--draw", altHeld && !drawMode && "eth-pr-grid--alt")}
      style={{ height }}
      data-testid="piano-roll-grid"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerLeave={onPointerLeave}
      onDoubleClick={onDoubleClick}
      onContextMenu={onContextMenu}
    >
      <canvas ref={rowsCanvasRef} className="eth-pr-grid__canvas" data-testid="piano-roll-rows-canvas" aria-hidden />
      {outside.map((o, i) => {
        const x0 = Math.max(0, beatsToPx(o.start, vp));
        const end = o.end === Infinity ? null : beatsToPx(o.end, vp);
        if (end !== null && end <= x0) return null;
        const last = !space.multi && o.end === Infinity;
        return (
          <div
            key={i}
            className="eth-pr-grid__outside"
            style={end === null ? { left: x0, right: 0 } : { left: x0, width: end - x0 }}
            data-testid={last ? "piano-roll-clip-end" : undefined}
          />
        );
      })}
      {space.multi &&
        space.bands.map((b) => (
          <div
            key={b.clip.id}
            className={clsx("eth-pr-grid__clip-start", b.clip.id === clip.id && "eth-pr-grid__clip-start--focus")}
            style={{ left: beatsToPx(b.start, vp), ["--eth-pr-clip-color" as string]: clipColors?.get(b.clip.id) }}
          />
        ))}
      {section && isSectionMarker(section) ? (
        <div className="eth-pr-grid__marker" data-testid="piano-roll-marker" data-beats={section.start} style={{ left: beatsToPx(section.start, vp) }} />
      ) : (
        section && (
          <div
            className={clsx("eth-pr-grid__section", section.pitches && "eth-pr-grid__section--pitches")}
            data-testid="piano-roll-section"
            style={{
              left: beatsToPx(section.start, vp),
              width: (section.end - section.start) * vp.pxPerBeat,
              ...(section.pitches && {
                top: pitchToY(section.pitches.high, keyH, rows),
                height: pitchToY(section.pitches.low, keyH, rows) + keyH - pitchToY(section.pitches.high, keyH, rows),
                bottom: "auto",
              }),
            }}
          />
        )
      )}
      <NotePitchCurves notes={visible} vp={vp} keyH={keyH} rowY={(p) => pitchToY(p, keyH, rows)} widthPx={widthPx} height={height} />
      <canvas ref={notesCanvasRef} className="eth-pr-grid__canvas eth-pr-grid__canvas--notes" data-testid="piano-roll-notes-canvas" aria-hidden />
      {velocityNote && <VelocityBadge note={notes.find((n) => n.id === velocityNote)} vp={vp} keyH={keyH} rows={rows} />}
      {marquee.rect && (
        <div
          className="eth-pr-grid__marquee"
          data-testid="piano-roll-marquee"
          style={{
            left: marquee.rect.x0,
            top: marquee.rect.y0,
            width: marquee.rect.x1 - marquee.rect.x0,
            height: marquee.rect.y1 - marquee.rect.y0,
          }}
        />
      )}
      <PlayheadLine view={view} mapping={space.fromSong} />
      {layers.map((Layer, i) => (
        <Layer
          key={i}
          clip={clip.id}
          gridRef={rootRef}
          mapping={cursorMapping}
          layoutKey={`${vp.pxPerBeat}:${vp.scrollBeats}:${keyH}:${rows.length}:${rows[0] ?? 0}`}
        />
      ))}
    </div>
  );
}

/** The grid's colors and sizes for the current theme (re-read on a theme switch). */
function useGridPalette(): GridPalette {
  const [theme] = useTheme();
  const [glow] = useGlow();
  const accent = useThemeColor("accent");
  const accentDim = useThemeColor("accentDim");
  const accentGlow = useThemeColor("accentGlow");
  const text = useThemeColor("text");
  const bgInset = useThemeColor("bgInset");
  const scaleRow = useThemeColor("pianoScaleRow");
  const scaleRootRow = useThemeColor("pianoScaleRootRow");
  const scaleRootLine = useThemeColor("pianoScaleRootLine");
  return useMemo(() => {
    const px = (name: `--${string}`, fallback: string) => parseFloat(readToken(name) || fallback) || parseFloat(fallback);
    return gridPalette({
      light: theme === "light",
      noGlow: !glow,
      accent: parseColor(accent),
      accentDim: parseColor(accentDim),
      accentGlow: parseColor(accentGlow),
      text: parseColor(text),
      bgInset: parseColor(bgInset),
      scaleRow: parseColor(scaleRow),
      scaleRootRow: parseColor(scaleRootRow),
      scaleRootLine: parseColor(scaleRootLine),
      white: parseColor(readToken("--eth-mix-white"), [1, 1, 1, 1]),
      black: parseColor(readToken("--eth-mix-black"), [0, 0, 0, 1]),
      radius: px("--eth-radius-xs", radius.xs),
      borderWidth: px("--eth-border-width", border.width),
      glowInner: px("--eth-size-selected-glow-inner", size.selectedGlowInner),
      glowOuter: px("--eth-size-selected-glow-outer", size.selectedGlowOuter),
      scaleOutNote: px("--eth-opacity-scale-out-note", opacityTokens.scaleOutNote),
    });
  }, [theme, glow, accent, accentDim, accentGlow, text, bgInset, scaleRow, scaleRootRow, scaleRootLine]);
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
  const t = useT();
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
      <Badge tone="accent">{t("piano_roll.velocity", { value: midiVelocity(note.velocity) })}</Badge>
    </div>
  );
}

/** Several commands as one (inside the drag's gesture). */
function batch(label: string, key: TextKey, commands: Command[]): Command {
  return cmd("Edit", { type: "Batch", label, key, commands });
}
