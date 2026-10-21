/**
 * Piano roll: the MIDI note editor for the clip open in the detail editor
 * (`useEditedClipId()` from `@/state`). The time axis is the clip's content timeline.
 *
 * - Keyboard gutter (click a key: select its notes and hear the pitch), note grid, and the lane area
 *   (`@/features/expression`): velocity, clip expression lanes (CC, pitch bend, channel
 *   pressure) and per-note pressure, picked from the lane bar.
 * - Draw: double-click empty space (keep holding and drag to set the length), or drag in
 *   draw mode (B); a new note takes the velocity of the last clicked, drawn or
 *   velocity-edited note (`lastVelocity.ts`). Move: drag a note body
 *   (vertical = pitch; Alt pressed mid-move: no snapping). Resize: drag either edge. Delete: double-click
 *   a note, or Delete/Backspace. Velocity: Alt/Option-drag a note body vertically (the
 *   selection, or just that note, by the same amount; Shift: fine), or the velocity lane.
 *   Alt bypasses snapping on every drag (move, draw, resize, marquee, ruler, stretch bar).
 *   Every drag is one undo gesture.
 * - Preview (headphones toggle, on by default, persisted): pressing a note or a key plays it
 *   on the clip's track instrument while held (`notePreview.ts`).
 * - Selection: click / shift / cmd-ctrl, marquee on empty space, cmd-A. Cmd/ctrl-drag a
 *   note duplicates the selection (copies follow the pointer).
 * - Keys: arrows nudge (shift = octave), cmd-U quantize, Esc deselects. Quantize (button,
 *   cmd-U) uses the settings of the groove Quantize… popover.
 * - Sections (section-edit): a marquee or a drag on the strip under the ruler selects a time
 *   range; cmd-C / X / V / D copy, cut, paste and duplicate notes with the section's exact
 *   length, gaps included (see `section.ts`). The desktop Edit menu's copy/cut/paste too.
 * - Stretch: a bar on the ruler spans the selection; drag its edges to time-scale
 *   the notes, its body to move them; ×2 / ÷2 in the toolbar (see `stretch.ts`).
 * - Multi-clip editing (Ableton-style): with several MIDI clips selected in the
 *   arrangement, opening one of them edits them all at once on the song timeline, as if they
 *   were one clip (see `multiClip.ts`). A strip under the ruler shows each clip's band (name,
 *   loop); a click on a band or a note focuses its clip (the toolbar, scale and expression
 *   lanes follow the focused clip).
 */

import { useMobileUi } from "@/features/mobile/mode";
import { contributions, useContributions } from "@/ext";
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import clsx from "clsx";
import type { Beats, Clip, ClipId, Command, MusicalScale, TrackScale } from "@/generated";
import { CHROMATIC_SCALE, resolveScale } from "@/domain/scales";
import { reverseNotesCommand } from "@/features/clip-editing/clipEditing";
import { Headphones } from "lucide-react";
import { Button, IconButton, Select, setDragCursor } from "@/kit";
import { ExpressionLanes } from "@/features/expression";
import { firstMatch, shortcutLabel } from "@/features/keymap";
import { GrooveControls, grooveMenuItems, grooveQuantizeCommand, useGrooveSettings } from "@/features/groove";
import { clipDisplayName, useClip, useEditedClipId, useEditorStore, useProjectStore } from "@/state";
import {
  beatsToPx,
  createTimelineViewStore,
  itemSelection,
  pxToBeats,
  resolveGrid,
  Ruler,
  snapToGrid,
  stepLength,
  useMiddleButtonPan,
  useSelectedItems,
  useTempoMap,
  useTimelineView,
  useTimelineWheel,
  useViewport,
  type GridSetting,
  type TimelineViewStore,
} from "@/timeline";
import { cmd, TransportContext, useConnectionStatus, useTransport } from "@/transport";
import { bindPlayFrom, selectionCursor, setPlayStart } from "@/features/time-edits/marker";
import { clipTempoMap, contentEnd } from "./clipTime";
import { colorCss } from "@/features/arrangement/helpers";
import { multiClipTransport, multiPasteCommand } from "./multiClip";
import { multiClipSpace, singleClipSpace, useMultiClipSelection, useSpaceNotes, type RollSpace } from "./rollSpace";
import { useSend } from "./drag";
import { createPitchRows, DEFAULT_KEY_HEIGHT, KEYBOARD_WIDTH, pitchToY, VELOCITY_LANE_HEIGHT, yToPitch } from "./geometry";
import { Keyboard } from "./Keyboard";
import { usePreviewSetting } from "./notePreview";
import { NoteGrid } from "./NoteGrid";
import { nudgeEdits } from "./noteEdits";
import { GRID_OPTIONS } from "./gridOptions";
import { useKeyHeightZoom } from "./useKeyHeightZoom";
import { useTouchPianoRoll } from "@/features/mobile/touchPianoRoll";
import { VelocityLane } from "./VelocityLane";
import { ScaleControls } from "./ScaleControls";
import { StretchBar, StretchButtons } from "./StretchBar";
import {
  clearSection,
  clearCommands,
  copyOf,
  editSource,
  isSectionMarker,
  notesIn,
  notesOver,
  sectionStep,
  pasteAt,
  pasteCommand,
  rangeOfSection,
  sectionOf,
  type NoteClipboard,
  usePianoRollSection,
} from "./section";
import "./pianoRoll.css";
import { useT } from "@/i18n";
import type { StepLabel } from "@/features/arrangement/editMath";
import { BeatGrid, useClipDrumRack } from "./BeatGrid";
import { useBeatView } from "./beatView";
import { EditorSwitch } from "./EditorSwitch";

/** Key-height change per pixel dragged on the zoom grip (exponential: ×e per 100 px). */
const ZOOM_GRIP_RATE = 0.01;

/** Prop-less piano roll mounted by the app shell. */
export function PianoRoll() {
  const id = useEditedClipId();
  const clip = useClip(id);
  // Several MIDI clips selected with the open one: edit them all (keyed by the set, so a
  // focus change inside it keeps the view).
  const clips = useMultiClipSelection(clip);
  // A Drum Rack track's clips open in the step sequencer (unless switched to the piano roll).
  const rack = useClipDrumRack(clip);
  const pianoRollChosen = useBeatView((s) => (clip ? s.pianoRoll.has(clip.track) : false));
  const t = useT();
  const pending = usePendingClip();
  if (!clip) return <Empty text={t("piano_roll.empty.no_clip")} />;
  if (clip.content.type !== "Midi") return <Empty text={t("piano_roll.empty.audio_clip")} />;
  // An extension's clip whose notes aren't here yet (`@/ext` "clips.pending"): it waits.
  const waiting = pending((clips ?? [clip]).find((c) => pending(c) !== null) ?? clip);
  if (waiting !== null) return <Empty text={waiting} loading />;
  if (rack && !clips && !pianoRollChosen) return <BeatGrid key={clip.id} clip={clip} rack={rack} />;
  if (clips) return <PianoRollEditor key={`multi:${clips.map((c) => c.id).join(",")}`} clip={clip} clips={clips} />;
  return <PianoRollEditor key={clip.id} clip={clip} />;
}

function Empty({ text, loading }: { text: string; loading?: boolean }) {
  return (
    <div className="eth-pr eth-pr--empty" data-testid={loading ? "piano-roll-loading" : "piano-roll-empty"} aria-busy={loading || undefined}>
      {text}
    </div>
  );
}

const FALLBACK_STEP_BEATS: Beats = 0.25;

/** The piano roll's keymap actions (piano-roll scope), in lookup order. */
const PIANO_ROLL_ACTIONS = [
  "edit.delete",
  "edit.deleteModified",
  "edit.selectAll",
  "pianoRoll.quantize",
  "clip.reverse",
  "edit.copy",
  "edit.cut",
  "edit.paste",
  "edit.duplicate",
  "pianoRoll.drawMode",
  "edit.deselect",
  "nudge.up",
  "nudge.down",
  "nudge.left",
  "nudge.right",
  "pianoRoll.octaveUp",
  "pianoRoll.octaveDown",
  "pianoRoll.nudgeLeftShift",
  "pianoRoll.nudgeUpAlt",
  "pianoRoll.nudgeDownAlt",
  "pianoRoll.nudgeRightShift",
] as const;

/** Nudge actions -> [grid steps, semitones]. */
const NUDGES: Partial<Record<string, readonly [number, number]>> = {
  "nudge.up": [0, 1],
  "nudge.down": [0, -1],
  "nudge.left": [-1, 0],
  "nudge.right": [1, 0],
  "pianoRoll.octaveUp": [0, 12],
  "pianoRoll.octaveDown": [0, -12],
  "pianoRoll.nudgeLeftShift": [-1, 0],
  "pianoRoll.nudgeUpAlt": [0, 1],
  "pianoRoll.nudgeDownAlt": [0, -1],
  "pianoRoll.nudgeRightShift": [1, 0],
};

/** The undo labels of a paste and a duplicate (their English, for agents and logs, and their message). */
const PASTE_NOTES: StepLabel = { text: "paste notes", key: { id: "undo.note.paste", args: {} } }; // i18n-ignore
const DUPLICATE_NOTES: StepLabel = { text: "duplicate notes", key: { id: "undo.note.duplicate_selection", args: {} } }; // i18n-ignore

/** A button tooltip with the action's current shortcut: "Draw mode (B)". */
const withKey = (label: string, action: string) => {
  const key = shortcutLabel(action);
  return key ? `${label} (${key})` : label;
};

export interface PianoRollEditorProps {
  /** The clip edited (the focused one when editing several). */
  clip: Clip;
  /** Several clips edited at once (multi-clip editing; must contain `clip`). */
  clips?: ReadonlyArray<Clip>;
  /** Zoom/scroll store (tests inject one with a fixed width). */
  view?: TimelineViewStore;
}

export function PianoRollEditor(props: PianoRollEditorProps) {
  const { clip, clips } = props;
  const multi = !!clips && clips.length > 1;
  const space = useMemo(() => (multi ? multiClipSpace(clip, clips) : singleClipSpace(clip)), [multi, clip, clips]);
  // Several clips: the editor's components keep sending commands in song beats, mapped to
  // each clip's content by a transport wrapper (`multiClipTransport`).
  const transport = useTransport();
  const connection = useConnectionStatus();
  // The wrapper reads the current clips when it sends (the set is fixed: the editor is keyed
  // by it).
  const idsKey = multi ? clips.map((c) => c.id).join(",") : "";
  const mapped = useMemo(() => {
    if (!idsKey) return null;
    const ids = idsKey.split(",");
    return multiClipTransport(transport, () => {
      const project = useProjectStore.getState().project;
      const current = new Map<ClipId, Clip>();
      for (const id of ids) {
        const c = project?.clips[id];
        if (c) current.set(id, c);
      }
      return { clips: current };
    });
  }, [idsKey, transport]);
  const context = useMemo(() => (mapped ? { transport: mapped, connection } : null), [mapped, connection]);
  const editor = <Editor space={space} view={props.view} />;
  return context ? <TransportContext.Provider value={context}>{editor}</TransportContext.Provider> : editor;
}

function Editor({ space, view: injectedView }: { space: RollSpace; view?: TimelineViewStore }) {
  const t = useT();
  const clip = space.focus;
  // Unnamed, the clip reads as its track.
  const clipName = useProjectStore((s) => (s.project ? clipDisplayName(s.project, clip) : clip.name)) || t("piano_roll.clip_fallback");
  const drumRack = useClipDrumRack(clip);
  const transport = useTransport();
  // A marker placed while playing becomes the play start on the next stop (idempotent with
  // the arrangement's binding: the first subscriber consumes it).
  useEffect(() => bindPlayFrom(transport), [transport]);
  const send = useSend();
  const ownView = useMemo(() => createTimelineViewStore({ pxPerBeat: 40, followPlayhead: false }), []);
  const view = injectedView ?? ownView;
  const vp = useViewport(view);
  const widthPx = useTimelineView(view, (s) => s.widthPx);
  const songTempo = useTempoMap();
  // One clip: its content axis (bar 1 at content 0); several: the song's tempo map.
  const tempo = useMemo(() => (space.multi ? songTempo : clipTempoMap(songTempo, clip)), [space.multi, songTempo, clip]);
  const { notes, ghosts } = useSpaceNotes(space);
  const selectedIds = useSelectedItems("note");
  const selected = useMemo(() => notes.filter((n) => selectedIds.has(n.id)), [notes, selectedIds]);

  const [gridIndex, setGridIndex] = useState(1);
  const [triplet, setTriplet] = useState(false);
  const [drawMode, setDrawMode] = useState(false);
  const mobile = useMobileUi((m) => m.enabled);
  const previewOn = usePreviewSetting((s) => s.enabled);
  // Scale: document state (project / track); highlight and folding are local view state.
  const projectScale = useProjectStore((s) => s.project?.settings.scale ?? CHROMATIC_SCALE);
  const trackScale = useProjectStore((s) => s.project?.tracks[clip.track]?.scale);
  const mode = trackScale?.type ?? "FollowProject";
  const scale = resolveScale(projectScale, trackScale);
  const [highlight, setHighlight] = useState(true);
  const [scaleOnly, setScaleOnly] = useState(false);
  const rows = useMemo(() => createPitchRows(scale, scaleOnly), [scale, scaleOnly]);
  const shownNotes = useMemo(() => notes.filter((n) => rows.includes(n.pitch)), [notes, rows]);
  // Multi-clip: the focused clip's notes in its content beats (the expression lanes').
  const focusNotes = useMemo(
    () => (space.multi ? shownNotes.filter((n) => n.clip === clip.id).map((n) => ({ ...n, start: n.start - space.focusShift })) : shownNotes),
    [space, shownNotes, clip.id],
  );
  const shownGhosts = useMemo(() => ghosts.filter((g) => rows.includes(g.note.pitch)), [ghosts, rows]);
  const clipColors = useClipColors(space);
  const setTrackScale = (value: TrackScale) => void send(cmd("Track", { type: "SetScale", id: clip.track, scale: value }));
  const setScale = (value: MusicalScale) => {
    if (mode === "FollowProject") void send(cmd("Project", { type: "SetScale", scale: value }));
    else setTrackScale({ type: "Custom", scale: value });
  };

  const grid: GridSetting = useMemo(() => {
    const g = GRID_OPTIONS[gridIndex]!.setting;
    return g.type === "Off" ? g : { ...g, triplet };
  }, [gridIndex, triplet]);
  const step = resolveGrid(grid, vp.pxPerBeat, tempo.signatureAt(0));
  const stepBeats = step ? stepLength(step, tempo.signatureAt(0)) : FALLBACK_STEP_BEATS;

  const rootRef = useRef<HTMLDivElement>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  // Keep the pre-fold scroll position: shrinking the canvas may clamp the DOM's scrollTop.
  const scrollTopRef = useRef(0);
  const laneRef = useRef<HTMLDivElement>(null);
  const [keyH, onVerticalZoom, setKeyHeight] = useKeyHeightZoom(bodyRef);
  /** The grip left of the keys (like Ableton's): drag down for taller keys, up for shorter;
   *  the key under the pointer stays put. Double-click restores the default height. */
  const onZoomGripDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    const body = bodyRef.current;
    if (e.button !== 0 || !body) return;
    e.preventDefault();
    e.stopPropagation();
    const top = body.getBoundingClientRect().top;
    const pointerY = e.clientY - top;
    const startK = keyH;
    const keysAtPointer = (body.scrollTop + pointerY) / startK;
    const startY = e.clientY;
    setDragCursor("ns-resize");
    const move = (ev: PointerEvent) => setKeyHeight(startK * Math.exp((ev.clientY - startY) * ZOOM_GRIP_RATE), keysAtPointer, pointerY);
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
      setDragCursor(null);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
  };
  useTimelineWheel(bodyRef, view, { smoothScrollY: true, onVerticalZoom, originPx: KEYBOARD_WIDTH });
  useTimelineWheel(laneRef, view);
  useMiddleButtonPan(bodyRef, view);
  useMiddleButtonPan(laneRef, view);
  // The mobile UI: pan, pinch, taps and note drags on touch screens.
  useTouchPianoRoll(bodyRef, view, KEYBOARD_WIDTH, onVerticalZoom);
  // Folding rows keeps the pitch at the middle of the view in place.
  const previousRows = useRef(rows);
  useLayoutEffect(() => {
    const body = bodyRef.current;
    if (body && previousRows.current !== rows) {
      const pitch = yToPitch(scrollTopRef.current + body.clientHeight / 2, keyH, previousRows.current);
      body.scrollTop = Math.max(0, pitchToY(pitch, keyH, rows) + keyH / 2 - body.clientHeight / 2);
      scrollTopRef.current = body.scrollTop;
    }
    previousRows.current = rows;
  }, [rows, keyH]);

  // The axis starts at the space's floor: before content 0 when the clip was extended to the
  // left past its content start (a leading gap, shown and editable like the rest).
  useEffect(() => {
    view.getState().setLimits({ minScrollBeats: space.floor });
  }, [view, space.floor]);

  // An extension's follow mode (`@/ext` "pianoRoll.follow"): publish this view, apply a followed
  // one (when it shows this clip).
  const followLayout = useRef({ clip: clip.id, shift: space.focusShift, rows, keyH });
  useLayoutEffect(() => {
    followLayout.current = { clip: clip.id, shift: space.focusShift, rows, keyH };
  });
  const follows = contributions("pianoRoll.follow");
  const headers = useContributions("pianoRoll.header");

  // Fit the clip horizontally and center its notes vertically when it opens (not while its
  // view follows someone's).
  const fitted = useRef(false);
  useEffect(() => {
    if (fitted.current || widthPx <= 0) return;
    if (follows.some((f) => f.following(clip.id))) return;
    fitted.current = true;
    if (space.multi) {
      const start = Math.min(...space.bands.map((b) => b.start));
      view.getState().zoomToRange({ start, end: Math.max(...space.bands.map((b) => b.end), start + 4) }, 8);
    } else {
      view.getState().zoomToRange({ start: space.floor, end: Math.max(contentEnd(clip), space.floor + 4) }, 8);
    }
    const body = bodyRef.current;
    if (body) {
      const pitches = notes.map((n) => n.pitch);
      const center = pitches.length ? (Math.min(...pitches) + Math.max(...pitches)) / 2 : 60;
      body.scrollTop = pitchToY(Math.round(center), keyH, rows) - body.clientHeight / 2;
      scrollTopRef.current = body.scrollTop;
    }
  }, [widthPx, view, clip, space, notes, keyH, rows]);

  // section-edit: the time range (of this editor) the clipboard/duplicate edits act on.
  // A zero-length section is the insert marker (⌘V target); `range` is the real section.
  const section = usePianoRollSection((s) => (s.section?.clip === space.key ? s.section : null));
  const range = rangeOfSection(section);

  /** Copy / cut / paste / duplicate (see `section.ts`); one undo step each. */
  const sectionEdit = async (kind: "copy" | "cut" | "paste" | "duplicate") => {
    const st = usePianoRollSection.getState();
    const sec = st.section?.clip === space.key ? st.section : null;
    const pasteAndSelect = async (r: ReturnType<typeof pasteCommand>) => {
      if (!r.command || !(await send(r.command))) return;
      itemSelection.getState().select("note", r.ids, "replace");
      usePianoRollSection.getState().setSection(r.section);
    };
    // Several clips: notes land in the clips under them (`multiPasteCommand`), never grow one.
    const paste = (cb: NoteClipboard, at: Beats, label: StepLabel) =>
      space.multi ? multiPasteCommand(space.bands, cb, at, clip.track, label) : pasteCommand(clip, cb, at, label);
    if (kind === "paste") {
      if (st.clipboard)
        await pasteAndSelect(paste(st.clipboard, space.multi ? (sec?.end ?? selectionCursor()) : pasteAt(clip, sec, selectionCursor()), PASTE_NOTES));
      return;
    }
    // Selected notes without a section: their exact span (a duplicate lands right at its end).
    const source = editSource(shownNotes, itemSelection.getState().selected.note, rangeOfSection(sec), null);
    if (!source) return;
    if (kind === "duplicate") {
      await pasteAndSelect(paste(copyOf(source), source.end, DUPLICATE_NOTES));
      return;
    }
    st.setClipboard(copyOf(source));
    if (kind === "cut") {
      // Only what is inside the section goes (notes crossing its edges are cut there).
      const commands = clearCommands(source);
      if (!commands.length) return;
      itemSelection.getState().select(
        "note",
        source.notes.map((n) => n.id),
        "remove",
      );
      await send(cmd("Edit", { type: "Batch", label: "cut notes", key: { id: "undo.note.cut", args: {} }, commands })); // i18n-ignore
    }
  };
  const sectionEditRef = useRef(sectionEdit);
  useEffect(() => {
    sectionEditRef.current = sectionEdit;
  });

  // The desktop app's Edit menu takes cmd-C/X/V before the page sees the key and sends
  // clipboard events instead: take them while the piano roll has the focus.
  useEffect(() => {
    const onClipboard = (e: ClipboardEvent) => {
      const root = rootRef.current;
      const active = document.activeElement;
      if (!root || !root.contains(active) || isTextEntry(active)) return;
      e.preventDefault();
      void sectionEditRef.current(e.type as "copy" | "cut" | "paste");
    };
    document.addEventListener("copy", onClipboard);
    document.addEventListener("cut", onClipboard);
    document.addEventListener("paste", onClipboard);
    return () => {
      document.removeEventListener("copy", onClipboard);
      document.removeEventListener("cut", onClipboard);
      document.removeEventListener("paste", onClipboard);
    };
  }, []);

  /** Drag on the strip under the ruler: select a section (and every shown note in it). */
  const onStripPointerDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    const box = e.currentTarget.getBoundingClientRect();
    const at = (ev: { clientX: number; altKey: boolean }) =>
      snapToGrid(Math.max(space.floor, pxToBeats(ev.clientX - box.left, view.getState())), ev.altKey ? null : sectionStep(step), tempo, "nearest");
    const from = at(e);
    const update = (ev: PointerEvent) => {
      const drawn = sectionOf(space.key, from, at(ev), undefined, space.floor);
      const inside = drawn ? notesOver(shownNotes, drawn.start, drawn.end) : [];
      usePianoRollSection.getState().setSection(drawn);
      itemSelection.getState().select(
        "note",
        inside.map((n) => n.id),
        "replace",
      );
    };
    const up = (ev: PointerEvent) => {
      window.removeEventListener("pointermove", update);
      window.removeEventListener("pointerup", up);
      update(ev);
      // The section start is the selection cursor (the playhead follows while stopped).
      const sec = usePianoRollSection.getState().section;
      if (sec?.clip === space.key) setPlayStart(transport, space.toSong(sec.start));
    };
    window.addEventListener("pointermove", update);
    window.addEventListener("pointerup", up);
  };

  const quantize = () =>
    void send(
      grooveQuantizeCommand(
        clip.id,
        selected.map((n) => n.id),
        useGrooveSettings.getState().quantize,
        stepBeats,
      ),
    );

  /** Mirror the selected notes (else the notes in the time section) in time; see `reverseNotesCommand`. */
  const reverseNotes = () => {
    const targets = selected.length ? selected : range ? notesIn(shownNotes, range.start, range.end) : [];
    return reverseNotesCommand(targets);
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    // Already handled by a focused control (e.g. arrows opening a toolbar Select).
    if (e.defaultPrevented) return;
    // keymap: chords from the user's keymap (piano-roll scope; see the registry).
    const action = firstMatch(PIANO_ROLL_ACTIONS, e);
    let command: Command | null = null;
    if (action === "edit.delete" || action === "edit.deleteModified") {
      // With a section: what is inside it (its selected notes, else all, cut at its edges);
      // without one, the selected notes.
      const source = range ? editSource(shownNotes, itemSelection.getState().selected.note, range, null) : null;
      if (source) {
        const commands = clearCommands(source);
        if (commands.length) command = cmd("Edit", { type: "Batch", label: "delete notes", key: { id: "undo.note.delete_section", args: {} }, commands }); // i18n-ignore
      } else if (selected.length) command = cmd("Note", { type: "Remove", ids: selected.map((n) => n.id) });
    } else if (action === "edit.selectAll") {
      // Like a plain (no Shift) marquee over every shown note: the selection is replaced and
      // no section is made (the old one goes), so edits act on the selected notes.
      clearSection();
      itemSelection.getState().select(
        "note",
        shownNotes.map((n) => n.id),
        "replace",
      );
    } else if (action === "pianoRoll.quantize") {
      quantize();
    } else if (action === "clip.reverse") {
      command = reverseNotes();
    } else if (action === "edit.copy" || action === "edit.cut" || action === "edit.paste" || action === "edit.duplicate") {
      void sectionEdit(action === "edit.copy" ? "copy" : action === "edit.cut" ? "cut" : action === "edit.paste" ? "paste" : "duplicate");
    } else if (action === "pianoRoll.drawMode") {
      setDrawMode((d) => !d);
    } else if (action === "edit.deselect") {
      itemSelection.getState().clear("note");
      clearSection();
    } else if (action && NUDGES[action] && selected.length) {
      const [dt, dp] = NUDGES[action];
      const edits = dp !== 0 ? nudgeEdits(selected, 0, dp) : nudgeEdits(selected, dt * stepBeats, 0, space.limits);
      command = cmd("Note", { type: "Edit", edits });
    } else {
      return;
    }
    e.preventDefault();
    e.stopPropagation();
    if (command) void send(command);
  };

  const locate = (beats: Beats) => {
    transport.send(cmd("Transport", { type: "Locate", position: space.toSong(beats) })).catch(() => {});
  };

  const loop = clip.looping;
  const mapping = space.fromSong;
  // The expression lanes draw the focused clip's content axis: shift the viewport under them.
  const laneVp = useMemo(() => (space.multi ? { ...vp, scrollBeats: vp.scrollBeats - space.focusShift } : vp), [space, vp]);

  return (
    <div
      ref={rootRef}
      className="eth-pr"
      tabIndex={0}
      role="group"
      aria-label={t("piano_roll.label")}
      data-testid="piano-roll"
      onKeyDown={onKeyDown}
      data-keymap-scope="pianoRoll"
      onPointerDownCapture={() => rootRef.current?.focus({ preventScroll: true })}
    >
      <div className="eth-pr__toolbar">
        <span className="eth-pr__title eth-user-text" title={clipName}>
          {clipName}
        </span>
        {space.multi && (
          <span className="eth-pr__clip-count" data-testid="piano-roll-clip-count" title={t("piano_roll.clip_count_title")}>
            {t("piano_roll.clip_count", { count: space.clips.length })}
          </span>
        )}
        {headers.map((Header, i) => (
          <Header key={i} clip={clip.id} />
        ))}
        {follows.map(({ Follow }, i) => (
          <Follow key={i} clip={clip.id} layout={followLayout} view={view} body={bodyRef} />
        ))}
        <span className="eth-pr__grid-select">
          {t("piano_roll.grid")}
          <Select
            size="sm"
            aria-label={t("piano_roll.grid")}
            value={String(gridIndex)}
            options={GRID_OPTIONS.map((o, i) => ({ value: String(i), label: o.label(), short: o.short?.() }))}
            onChange={(v) => setGridIndex(Number(v))}
          />
        </span>
        <Button size="sm" active={triplet} onClick={() => setTriplet((t) => !t)} title={t("piano_roll.triplet_grid")}>
          3
        </Button>
        <IconButton
          size="sm"
          tone="ghost"
          className="eth-pr__preview"
          data-testid="piano-roll-preview"
          label={previewOn ? t("piano_roll.preview_on") : t("piano_roll.preview_off")}
          active={previewOn}
          icon={<Headphones />}
          onClick={() => usePreviewSetting.getState().setEnabled(!previewOn)}
        />
        {/* Touch (the mobile shell): no B key, so draw mode has its button. */}
        {mobile && (
          <Button size="sm" active={drawMode} onClick={() => setDrawMode((d) => !d)} title={t("piano_roll.draw_mode")}>
            {t("piano_roll.draw")}
          </Button>
        )}
        <Button
          size="sm"
          data-testid="piano-roll-reverse"
          disabled={selected.length === 0 && !range}
          onClick={() => {
            const c = reverseNotes();
            if (c) void send(c);
          }}
          title={withKey(t("piano_roll.reverse_title"), "clip.reverse")}
        >
          {t("piano_roll.reverse")}
        </Button>
        <StretchButtons sectionKey={space.key} clip={space.grow} notes={shownNotes} />
        <GrooveControls
          clip={clip.id}
          selected={selected.map((n) => n.id)}
          rollStep={stepBeats}
          quantizeTitle={withKey(t("piano_roll.quantize_title"), "pianoRoll.quantize")}
        />
        <ScaleControls
          scale={scale}
          mode={mode}
          onScale={setScale}
          onMode={(type) => setTrackScale(type === "Custom" ? { type, scale } : { type })}
          highlight={highlight}
          onHighlight={setHighlight}
          only={scaleOnly}
          onOnly={setScaleOnly}
        />
        {drumRack && !space.multi && <EditorSwitch track={clip.track} />}
      </div>

      <div className="eth-pr__header">
        <div className="eth-pr__corner" style={{ width: KEYBOARD_WIDTH }} />
        <div className="eth-pr__ruler">
          <Ruler view={view} showLoop={false} tempo={tempo} grid={grid} playheadMapping={mapping} onLocate={locate} syncWidth={!injectedView} />
          <StretchBar sectionKey={space.key} clip={space.grow} notes={shownNotes} vp={vp} step={step} tempo={tempo} />
          {space.multi && <ClipBands space={space} vp={vp} colors={clipColors} />}
          <div className="eth-pr__loopbar" data-testid="piano-roll-loopbar" onPointerDown={onStripPointerDown} title={t("piano_roll.section_strip")}>
            {range && (
              <div
                className="eth-pr__section"
                data-testid="piano-roll-strip-section"
                style={{ left: beatsToPx(range.start, vp), width: (range.end - range.start) * vp.pxPerBeat }}
              />
            )}
            {section && isSectionMarker(section) && <div className="eth-pr__marker" style={{ left: beatsToPx(section.start, vp) }} />}
            {!space.multi && loop.enabled && (
              <div
                className="eth-pr__loop"
                data-testid="piano-roll-loop"
                style={{ left: beatsToPx(loop.start, vp), width: (loop.end - loop.start) * vp.pxPerBeat }}
                title={t("piano_roll.clip_loop")}
              />
            )}
            {!space.multi && <div className="eth-pr__start" style={{ left: beatsToPx(clip.offset, vp) }} title={t("piano_roll.clip_start")} />}
          </div>
        </div>
      </div>

      <div
        ref={bodyRef}
        className="eth-pr__body"
        onScroll={(e) => {
          scrollTopRef.current = e.currentTarget.scrollTop;
        }}
      >
        <div className="eth-pr__canvas">
          <div
            className="eth-pr__zoom-grip"
            style={{ height: rows.length * keyH }}
            role="separator"
            aria-orientation="horizontal"
            aria-label={t("piano_roll.key_height")}
            title={t("piano_roll.key_height_title")}
            onPointerDown={onZoomGripDown}
            onDoubleClick={() => {
              const body = bodyRef.current;
              if (!body) return;
              const pointerY = body.clientHeight / 2;
              setKeyHeight(DEFAULT_KEY_HEIGHT, (body.scrollTop + pointerY) / keyH, pointerY);
            }}
          />
          <Keyboard keyH={keyH} notes={shownNotes} rows={rows} scale={scale} highlight={highlight} track={clip.track} />
          <NoteGrid
            space={space}
            scrollRef={bodyRef}
            notes={shownNotes}
            ghosts={shownGhosts}
            clipColors={clipColors}
            rows={rows}
            scale={scale}
            highlight={highlight}
            view={view}
            vp={vp}
            widthPx={widthPx}
            keyH={keyH}
            tempo={tempo}
            step={step}
            newNoteBeats={stepBeats}
            drawMode={drawMode}
            section={section}
            menuItems={(ids) => [
              ...grooveMenuItems(clip.id, ids, stepBeats, send),
              "separator" as const,
              {
                label: t("piano_roll.reverse"),
                shortcut: shortcutLabel("clip.reverse"),
                onSelect: () => {
                  const c = reverseNotesCommand(notes.filter((n) => ids.includes(n.id)));
                  if (c) void send(c);
                },
              },
            ]}
          />
        </div>
      </div>

      <ExpressionLanes
        clip={clip}
        notes={focusNotes}
        vp={laneVp}
        widthPx={widthPx}
        height={VELOCITY_LANE_HEIGHT}
        stepBeats={stepBeats}
        labelWidth={KEYBOARD_WIDTH}
        bodyRef={laneRef}
        velocity={<VelocityLane notes={shownNotes} vp={vp} widthPx={widthPx} />}
        view={view}
        envelopeOffset={space.multi ? space.focusShift : 0}
        tempo={tempo}
      />
    </div>
  );
}

/** Note color per clip in the multi-clip editor (the clip's color, else its track's). */
function useClipColors(space: RollSpace): ReadonlyMap<ClipId, string> | undefined {
  const tracks = useProjectStore((s) => s.project?.tracks);
  return useMemo(() => {
    if (!space.multi || !tracks) return undefined;
    const colors = new Map<ClipId, string>();
    for (const c of space.clips) {
      const color = c.color ?? tracks[c.track]?.color;
      if (color !== undefined && color !== null) colors.set(c.id, colorCss(color));
    }
    return colors;
  }, [space, tracks]);
}

/**
 * The clips' bands under the ruler (multi-clip editing): each clip's name and region in its
 * color, its loop region, the focused clip highlighted. A click focuses the clip.
 */
function ClipBands({ space, vp, colors }: { space: RollSpace; vp: { pxPerBeat: number; scrollBeats: number }; colors?: ReadonlyMap<ClipId, string> }) {
  const t = useT();
  const tracks = useProjectStore((s) => s.project?.tracks);
  return (
    <div className="eth-pr__bands" data-testid="piano-roll-clip-bands">
      {space.bands.map((b) => {
        const focused = b.clip.id === space.focus.id;
        const track = tracks?.[b.clip.track]?.name;
        const name = b.clip.name || track || t("piano_roll.clip_fallback");
        return (
          <button
            type="button"
            key={b.clip.id}
            className={clsx("eth-pr__band", focused && "eth-pr__band--focus")}
            data-testid="piano-roll-clip-band"
            data-clip-id={b.clip.id}
            aria-pressed={focused}
            title={track ? (focused ? t("piano_roll.band", { name, track }) : t("piano_roll.band_focus", { name, track })) : name}
            style={{
              left: beatsToPx(b.start, vp),
              width: Math.max(2, (b.end - b.start) * vp.pxPerBeat),
              ["--eth-pr-clip-color" as string]: colors?.get(b.clip.id),
            }}
            onPointerDown={(e) => e.stopPropagation()}
            onClick={() => useEditorStore.getState().focusClip(b.clip.id)}
          >
            {b.loop && (
              <span
                className="eth-pr__band-loop"
                data-testid="piano-roll-band-loop"
                style={{ left: (b.loop.start - b.start) * vp.pxPerBeat, width: (b.loop.end - b.loop.start) * vp.pxPerBeat }}
              />
            )}
            <span className="eth-pr__band-name eth-user-text">{name}</span>
          </button>
        );
      })}
    </div>
  );
}

function isTextEntry(target: Element | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName);
}

/**
 * The extensions' pending clips (`@/ext` "clips.pending", hooks): for a clip, the editor's
 * text while its content is on its way, or `null`.
 */
function usePendingClip(): (clip: Clip) => string | null {
  const checks = contributions("clips.pending").map((p) => ({ pending: p.usePending(), text: p.text }));
  return (clip) => checks.find((c) => c.pending(clip))?.text() ?? null;
}
