/**
 * Piano roll: the MIDI note editor for the clip open in the detail editor
 * (`useEditedClipId()` from `@/state`). The time axis is the clip's content timeline.
 *
 * - Keyboard gutter (click a key: select its notes), note grid, velocity lane.
 * - Draw: double-click empty space, or drag in draw mode (B). Move: drag a note body
 *   (vertical = pitch). Resize: drag either edge. Delete: double-click a note, or
 *   Delete/Backspace. Alt bypasses snapping. Every drag is one undo gesture.
 * - Selection: click / shift / cmd-ctrl, marquee on empty space, cmd-A.
 * - Keys: arrows nudge (shift = octave), cmd-U quantize, cmd-D duplicate, Esc deselects.
 */

import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { Beats, Clip, Command, Note, NoteId, MusicalScale, TrackScale } from "@/generated";
import { Button } from "@/kit";
import { useClip, useEditedClipId, useNotesOfClip, useProjectStore } from "@/state";
import {
  beatsToPx,
  createTimelineViewStore,
  formatGridStep,
  itemSelection,
  resolveGrid,
  Ruler,
  stepLength,
  useSelectedItems,
  useTempoMap,
  useTimelineView,
  useTimelineWheel,
  useViewport,
  type GridSetting,
  type TimelineViewStore,
} from "@/timeline";
import { cmd, newId, useTransport } from "@/transport";
import { clipTempoMap, contentEnd, contentToSong, songToContent } from "./clipTime";
import { useSend } from "./drag";
import { DEFAULT_KEY_HEIGHT, KEYBOARD_WIDTH, pitchToY, yToPitch, createPitchRows } from "./geometry";
import { Keyboard } from "./Keyboard";
import { NoteGrid } from "./NoteGrid";
import { nudgeEdits, quantizeCommand } from "./noteEdits";
import { GRID_OPTIONS } from "./gridOptions";
import { VelocityLane } from "./VelocityLane";
import "./pianoRoll.css";
import { CHROMATIC_SCALE, resolveScale } from "@/domain/scales";
import { ScaleControls } from "./ScaleControls";

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
  const keyH = DEFAULT_KEY_HEIGHT;
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
  useTimelineWheel(bodyRef, view);
  useTimelineWheel(laneRef, view);
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

  const quantize = () => void send(quantizeCommand(clip.id, selected.map((n) => n.id), stepBeats));

  const onKeyDown = (e: React.KeyboardEvent) => {
    if ((e.target as HTMLElement).closest("select, input, button, textarea")) return;
    const mod = e.metaKey || e.ctrlKey;
    const key = e.key.toLowerCase();
    let command: Command | null = null;
    if (key === "delete" || key === "backspace") {
      if (selected.length) command = cmd("Note", { type: "Remove", ids: selected.map((n) => n.id) });
    } else if (mod && key === "a") {
      itemSelection.getState().select("note", notes.map((n) => n.id), "replace");
    } else if (mod && key === "u") {
      quantize();
    } else if (mod && key === "d") {
      if (selected.length) {
        const dup = duplicateCommand(selected);
        void send(dup.command).then((ok) => ok && itemSelection.getState().select("note", dup.ids, "replace"));
      }
    } else if (!mod && key === "b") {
      setDrawMode((d) => !d);
    } else if (key === "escape") {
      itemSelection.getState().clear("note");
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
      onPointerDownCapture={(e) => {
        if (!(e.target as HTMLElement).closest("select, input, button, textarea")) rootRef.current?.focus({ preventScroll: true });
      }}
    >
      <div className="eth-pr__toolbar">
        <span className="eth-pr__title" title={clip.name}>
          {clip.name || "MIDI Clip"}
        </span>
        <label className="eth-pr__grid-select">
          Grid
          <select value={gridIndex} onChange={(e) => setGridIndex(Number(e.target.value))} aria-label="Grid">
            {GRID_OPTIONS.map((o, i) => (
              <option key={o.label} value={i}>
                {o.label}
              </option>
            ))}
          </select>
        </label>
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
        <ScaleControls scale={scale} mode={mode} onScale={setScale}
          onMode={(type) => setTrackScale(type === "Custom" ? { type, scale } : { type })}
          highlight={highlight} onHighlight={setHighlight} only={scaleOnly} onOnly={setScaleOnly} />
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
          <div className="eth-pr__loopbar" data-testid="piano-roll-loopbar">
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

/** Duplicate the selection right after itself (Ableton cmd-D); returns the copies' ids. */
function duplicateCommand(selected: ReadonlyArray<Note>): { command: Command; ids: NoteId[] } {
  const start = Math.min(...selected.map((n) => n.start));
  const end = Math.max(...selected.map((n) => n.start + n.duration));
  const copies = selected.map((n) => ({ from: n.id, new_id: newId() }));
  return {
    command: cmd("Note", { type: "Duplicate", copies, offset: end - start, transpose: 0 }),
    ids: copies.map((c) => c.new_id),
  };
}
