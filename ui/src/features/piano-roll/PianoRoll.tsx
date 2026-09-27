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
 * - Keys: arrows nudge (shift = octave), cmd-U quantize, cmd-D duplicate, Esc deselects.
 *   Quantize (button, cmd-U) uses the settings of the groove Quantize… popover.
 */

import { useEffect, useMemo, useRef, useState } from "react";
import type { Beats, Clip, Command, Note, NoteId } from "@/generated";
import { Button, Select } from "@/kit";
import { GrooveControls, grooveMenuItems, grooveQuantizeCommand, useGrooveSettings } from "@/features/groove";
import { useClip, useEditedClipId, useNotesOfClip } from "@/state";
import {
  beatsToPx,
  createTimelineViewStore,
  formatGridStep,
  itemSelection,
  resolveGrid,
  Ruler,
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
import { cmd, newId, useTransport } from "@/transport";
import { clipTempoMap, contentEnd, contentToSong, songToContent } from "./clipTime";
import { useSend } from "./drag";
import { KEYBOARD_WIDTH, pitchToY } from "./geometry";
import { Keyboard } from "./Keyboard";
import { NoteGrid } from "./NoteGrid";
import { nudgeEdits } from "./noteEdits";
import { GRID_OPTIONS } from "./gridOptions";
import { useKeyHeightZoom } from "./useKeyHeightZoom";
import { VelocityLane } from "./VelocityLane";
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

  const grid: GridSetting = useMemo(() => {
    const g = GRID_OPTIONS[gridIndex]!.setting;
    return g.type === "Off" ? g : { ...g, triplet };
  }, [gridIndex, triplet]);
  const step = resolveGrid(grid, vp.pxPerBeat, tempo.signatureAt(0));
  const stepBeats = step ? stepLength(step, tempo.signatureAt(0)) : FALLBACK_STEP_BEATS;

  const rootRef = useRef<HTMLDivElement>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const laneRef = useRef<HTMLDivElement>(null);
  const [keyH, onVerticalZoom] = useKeyHeightZoom(bodyRef);
  useTimelineWheel(bodyRef, view, { smoothScrollY: true, onVerticalZoom, originPx: KEYBOARD_WIDTH });
  useTimelineWheel(laneRef, view);
  useMiddleButtonPan(bodyRef, view);
  useMiddleButtonPan(laneRef, view);

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
      body.scrollTop = pitchToY(Math.round(center), keyH) - body.clientHeight / 2;
    }
  }, [widthPx, view, clip, notes, keyH]);

  const quantize = () =>
    void send(grooveQuantizeCommand(clip.id, selected.map((n) => n.id), useGrooveSettings.getState().quantize, stepBeats));

  const onKeyDown = (e: React.KeyboardEvent) => {
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
      onPointerDownCapture={() => rootRef.current?.focus({ preventScroll: true })}
    >
      <div className="eth-pr__toolbar">
        <span className="eth-pr__title" title={clip.name}>
          {clip.name || "MIDI Clip"}
        </span>
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

      <div ref={bodyRef} className="eth-pr__body">
        <div className="eth-pr__canvas">
          <Keyboard keyH={keyH} notes={notes} />
          <NoteGrid
            clip={clip}
            notes={notes}
            view={view}
            vp={vp}
            widthPx={widthPx}
            keyH={keyH}
            tempo={tempo}
            step={step}
            newNoteBeats={stepBeats}
            drawMode={drawMode}
            menuItems={(ids) => grooveMenuItems(clip.id, ids, stepBeats, send)}
          />
        </div>
      </div>

      <div className="eth-pr__lane">
        <div className="eth-pr__lane-label" style={{ width: KEYBOARD_WIDTH }}>
          Velocity
        </div>
        <div ref={laneRef} className="eth-pr__lane-body">
          <VelocityLane notes={notes} vp={vp} widthPx={widthPx} />
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
