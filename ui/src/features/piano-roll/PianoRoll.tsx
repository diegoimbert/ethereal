/**
 * Piano roll: the MIDI note editor for the clip open in the detail editor
 * (`useEditedClipId()` from `@/state`). The time axis is the clip's content timeline.
 *
 * - Keyboard gutter (click a key: select its notes), note grid, velocity lane.
 * - Draw: double-click empty space (keep holding and drag to set the length), or drag in
 *   draw mode (B). Move: drag a note body
 *   (vertical = pitch). Resize: drag either edge. Delete: double-click a note, or
 *   Delete/Backspace. Alt bypasses snapping. Every drag is one undo gesture.
 * - Selection: click / shift / cmd-ctrl, marquee on empty space, cmd-A. Cmd/ctrl-drag a
 *   note duplicates the selection (copies follow the pointer).
 * - Keys: arrows nudge (shift = octave), cmd-U quantize, Esc deselects. Quantize (button,
 *   cmd-U) uses the settings of the groove Quantize… popover.
 * - Sections (section-edit): a marquee or a drag on the strip under the ruler selects a time
 *   range; cmd-C / X / V / D copy, cut, paste and duplicate notes with the section's exact
 *   length, gaps included (see `section.ts`). The desktop Edit menu's copy/cut/paste too.
 */

import { useEffect, useLayoutEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import type { Beats, Clip, Command, MusicalScale, TrackScale } from "@/generated";
import { CHROMATIC_SCALE, resolveScale } from "@/domain/scales";
import { Button, Select } from "@/kit";
import { EditingPeers } from "@/features/collab/presence";
import { GrooveControls, grooveMenuItems, grooveQuantizeCommand, useGrooveSettings } from "@/features/groove";
import { useClip, useEditedClipId, useNotesOfClip, useProjectStore } from "@/state";
import {
  beatsToPx,
  createTimelineViewStore,
  formatGridStep,
  itemSelection,
  playheadBeats,
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
import { cmd, useTransport } from "@/transport";
import { bindPlayFrom } from "@/features/time-edits/marker";
import { clipTempoMap, contentEnd, contentToSong, songToContent } from "./clipTime";
import { useSend } from "./drag";
import { createPitchRows, KEYBOARD_WIDTH, pitchToY, yToPitch } from "./geometry";
import { Keyboard } from "./Keyboard";
import { NoteGrid } from "./NoteGrid";
import { nudgeEdits } from "./noteEdits";
import { GRID_OPTIONS } from "./gridOptions";
import { useKeyHeightZoom } from "./useKeyHeightZoom";
import { VelocityLane } from "./VelocityLane";
import { ScaleControls } from "./ScaleControls";
import {
  clearSection,
  copyOf,
  editSource,
  isSectionMarker,
  notesIn,
  pasteAt,
  pasteCommand,
  rangeOfSection,
  regionOf,
  sectionOf,
  usePianoRollSection,
} from "./section";
import "./pianoRoll.css";

/** Prop-less piano roll mounted by the app shell. */
export function PianoRoll() {
  const id = useEditedClipId();
  const clip = useClip(id);
  if (!clip) return <Empty text="Double-click a MIDI clip to edit its notes." />;
  if (clip.content.type !== "Midi") return <Empty text="The piano roll edits MIDI clips. This is an audio clip." />;
  return <PianoRollEditor key={clip.id} clip={clip} />;
}

function Empty({ text }: { text: string }) {
  return (
    <div className="eth-pr eth-pr--empty" data-testid="piano-roll-empty">
      {text}
    </div>
  );
}

const FALLBACK_STEP_BEATS: Beats = 0.25;

export interface PianoRollEditorProps {
  clip: Clip;
  /** Zoom/scroll store (tests inject one with a fixed width). */
  view?: TimelineViewStore;
}

export function PianoRollEditor({ clip, view: injectedView }: PianoRollEditorProps) {
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
  const tempo = useMemo(() => clipTempoMap(songTempo, clip), [songTempo, clip]);
  const notes = useNotesOfClip(clip.id);
  const selectedIds = useSelectedItems("note");
  const selected = useMemo(() => notes.filter((n) => selectedIds.has(n.id)), [notes, selectedIds]);

  const [gridIndex, setGridIndex] = useState(1);
  const [triplet, setTriplet] = useState(false);
  const [drawMode, setDrawMode] = useState(false);
  // Scale: document state (project / track); highlight and folding are local view state.
  const projectScale = useProjectStore((s) => s.project?.settings.scale ?? CHROMATIC_SCALE);
  const trackScale = useProjectStore((s) => s.project?.tracks[clip.track]?.scale);
  const mode = trackScale?.type ?? "FollowProject";
  const scale = resolveScale(projectScale, trackScale);
  const [highlight, setHighlight] = useState(true);
  const [scaleOnly, setScaleOnly] = useState(false);
  const rows = useMemo(() => createPitchRows(scale, scaleOnly), [scale, scaleOnly]);
  const shownNotes = useMemo(() => notes.filter((n) => rows.includes(n.pitch)), [notes, rows]);
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
  const [keyH, onVerticalZoom] = useKeyHeightZoom(bodyRef);
  useTimelineWheel(bodyRef, view, { smoothScrollY: true, onVerticalZoom, originPx: KEYBOARD_WIDTH });
  useTimelineWheel(laneRef, view);
  useMiddleButtonPan(bodyRef, view);
  useMiddleButtonPan(laneRef, view);
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

  // Fit the clip horizontally and center its notes vertically when it opens.
  const fitted = useRef(false);
  useEffect(() => {
    if (fitted.current || widthPx <= 0) return;
    fitted.current = true;
    view.getState().zoomToRange({ start: 0, end: Math.max(contentEnd(clip), 4) }, 8);
    const body = bodyRef.current;
    if (body) {
      const pitches = notes.map((n) => n.pitch);
      const center = pitches.length ? (Math.min(...pitches) + Math.max(...pitches)) / 2 : 60;
      body.scrollTop = pitchToY(Math.round(center), keyH, rows) - body.clientHeight / 2;
      scrollTopRef.current = body.scrollTop;
    }
  }, [widthPx, view, clip, notes, keyH, rows]);

  // section-edit: the time range (of this clip) the clipboard/duplicate edits act on.
  // A zero-length section is the insert marker (⌘V target); `range` is the real section.
  const section = usePianoRollSection((s) => (s.section?.clip === clip.id ? s.section : null));
  const range = rangeOfSection(section);

  /** Copy / cut / paste / duplicate (see `section.ts`); one undo step each. */
  const sectionEdit = async (kind: "copy" | "cut" | "paste" | "duplicate") => {
    const st = usePianoRollSection.getState();
    const sec = st.section?.clip === clip.id ? st.section : null;
    const pasteAndSelect = async (r: ReturnType<typeof pasteCommand>) => {
      if (!r.command || !(await send(r.command))) return;
      itemSelection.getState().select("note", r.ids, "replace");
      usePianoRollSection.getState().setSection(r.section);
    };
    if (kind === "paste") {
      if (st.clipboard) await pasteAndSelect(pasteCommand(clip, st.clipboard, pasteAt(clip, sec, playheadBeats()), "Paste Notes"));
      return;
    }
    const source = editSource(shownNotes, itemSelection.getState().selected.note, rangeOfSection(sec), step ? stepBeats : null);
    if (!source) return;
    if (kind === "duplicate") {
      await pasteAndSelect(pasteCommand(clip, copyOf(source), source.end, "Duplicate Notes"));
      return;
    }
    st.setClipboard(copyOf(source));
    if (kind === "cut" && source.notes.length) {
      const ids = source.notes.map((n) => n.id);
      itemSelection.getState().select("note", ids, "remove");
      await send(cmd("Note", { type: "Remove", ids }));
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
      snapToGrid(Math.max(0, pxToBeats(ev.clientX - box.left, view.getState())), ev.altKey ? null : step, tempo, "nearest");
    const from = at(e);
    const update = (ev: PointerEvent) => {
      const sec = sectionOf(clip.id, from, at(ev));
      usePianoRollSection.getState().setSection(sec);
      const inside = sec ? notesIn(shownNotes, sec.start, sec.end).map((n) => n.id) : [];
      itemSelection.getState().select("note", inside, "replace");
    };
    const up = (ev: PointerEvent) => {
      window.removeEventListener("pointermove", update);
      window.removeEventListener("pointerup", up);
      update(ev);
    };
    window.addEventListener("pointermove", update);
    window.addEventListener("pointerup", up);
  };

  const quantize = () =>
    void send(grooveQuantizeCommand(clip.id, selected.map((n) => n.id), useGrooveSettings.getState().quantize, stepBeats));

  const onKeyDown = (e: React.KeyboardEvent) => {
    // Already handled by a focused control (e.g. arrows opening a toolbar Select).
    if (e.defaultPrevented) return;
    const mod = e.metaKey || e.ctrlKey;
    const key = e.key.toLowerCase();
    let command: Command | null = null;
    if (key === "delete" || key === "backspace") {
      // The selected notes; with a section and no selection, the notes in the section.
      const doomed = selected.length ? selected : range ? notesIn(shownNotes, range.start, range.end) : [];
      if (doomed.length) command = cmd("Note", { type: "Remove", ids: doomed.map((n) => n.id) });
    } else if (mod && key === "a") {
      itemSelection.getState().select("note", shownNotes.map((n) => n.id), "replace");
      usePianoRollSection.getState().setSection(regionOf(clip));
    } else if (mod && key === "u") {
      quantize();
    } else if (mod && !e.shiftKey && !e.altKey && (key === "c" || key === "x" || key === "v" || key === "d")) {
      void sectionEdit(key === "c" ? "copy" : key === "x" ? "cut" : key === "v" ? "paste" : "duplicate");
    } else if (!mod && key === "b") {
      setDrawMode((d) => !d);
    } else if (key === "escape") {
      itemSelection.getState().clear("note");
      clearSection();
    } else if (key.startsWith("arrow") && selected.length) {
      const dir = key === "arrowup" || key === "arrowright" ? 1 : -1;
      const vertical = key === "arrowup" || key === "arrowdown";
      const edits = vertical ? nudgeEdits(selected, 0, dir * (e.shiftKey ? 12 : 1)) : nudgeEdits(selected, dir * stepBeats, 0);
      command = cmd("Note", { type: "Edit", edits });
    } else {
      return;
    }
    e.preventDefault();
    e.stopPropagation();
    if (command) void send(command);
  };

  const locate = (content: Beats) => {
    transport.send(cmd("Transport", { type: "Locate", position: contentToSong(clip, content) })).catch(() => {});
  };

  const loop = clip.looping;
  const mapping = useMemo(() => (song: Beats) => songToContent(clip, song), [clip]);

  return (
    <div
      ref={rootRef}
      className="eth-pr"
      tabIndex={0}
      data-testid="piano-roll"
      onKeyDown={onKeyDown}
      onPointerDownCapture={() => rootRef.current?.focus({ preventScroll: true })}
    >
      <div className="eth-pr__toolbar">
        <span className="eth-pr__title" title={clip.name}>
          {clip.name || "MIDI Clip"}
        </span>
        <EditingPeers clip={clip.id} />
        <span className="eth-pr__grid-select">
          Grid
          <Select
            size="sm"
            aria-label="Grid"
            value={String(gridIndex)}
            options={GRID_OPTIONS.map((o, i) => ({ value: String(i), label: o.label }))}
            onChange={(v) => setGridIndex(Number(v))}
          />
        </span>
        <Button size="sm" active={triplet} onClick={() => setTriplet((t) => !t)} title="Triplet grid">
          3
        </Button>
        <span className="eth-pr__step" data-testid="piano-roll-step">
          {formatGridStep(step)}
        </span>
        <Button size="sm" active={drawMode} onClick={() => setDrawMode((d) => !d)} title="Draw mode (B)">
          Draw
        </Button>
        <Button size="sm" onClick={quantize} title="Quantize to the grid (Cmd/Ctrl+U)">
          Quantize
        </Button>
        <GrooveControls clip={clip.id} selected={selected.map((n) => n.id)} rollStep={stepBeats} />
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
      </div>

      <div className="eth-pr__header">
        <div className="eth-pr__corner" style={{ width: KEYBOARD_WIDTH }} />
        <div className="eth-pr__ruler">
          <Ruler
            view={view}
            showLoop={false}
            tempo={tempo}
            grid={grid}
            playheadMapping={mapping}
            onLocate={locate}
            syncWidth={!injectedView}
          />
          <div className="eth-pr__loopbar" data-testid="piano-roll-loopbar" onPointerDown={onStripPointerDown} title="Drag to select a section">
            {range && (
              <div
                className="eth-pr__section"
                data-testid="piano-roll-strip-section"
                style={{ left: beatsToPx(range.start, vp), width: (range.end - range.start) * vp.pxPerBeat }}
              />
            )}
            {section && isSectionMarker(section) && <div className="eth-pr__marker" style={{ left: beatsToPx(section.start, vp) }} />}
            {loop.enabled && (
              <div
                className="eth-pr__loop"
                data-testid="piano-roll-loop"
                style={{ left: beatsToPx(loop.start, vp), width: (loop.end - loop.start) * vp.pxPerBeat }}
                title="Clip loop"
              />
            )}
            <div className="eth-pr__start" style={{ left: beatsToPx(clip.offset, vp) }} title="Clip start" />
          </div>
        </div>
      </div>

      <div ref={bodyRef} className="eth-pr__body" onScroll={(e) => { scrollTopRef.current = e.currentTarget.scrollTop; }}>
        <div className="eth-pr__canvas">
          <Keyboard keyH={keyH} notes={shownNotes} rows={rows} scale={scale} highlight={highlight} />
          <NoteGrid
            clip={clip}
            notes={shownNotes}
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
            menuItems={(ids) => grooveMenuItems(clip.id, ids, stepBeats, send)}
          />
        </div>
      </div>

      <div className="eth-pr__lane">
        <div className="eth-pr__lane-label" style={{ width: KEYBOARD_WIDTH }}>
          Velocity
        </div>
        <div ref={laneRef} className="eth-pr__lane-body">
          <VelocityLane notes={shownNotes} vp={vp} widthPx={widthPx} />
        </div>
      </div>
    </div>
  );
}

function isTextEntry(target: Element | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName);
}
