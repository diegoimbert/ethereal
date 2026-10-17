/**
 * The step sequencer (FL Studio's "channel rack" steps) that replaces the piano roll on a
 * track with a Drum Rack: one row per pad (its name, or its note: "C1", "C#1", ...), one cell
 * per 16th step across the clip's content. Left click toggles a hit, right click removes it;
 * drag along a row to paint (or erase: from a hit, or with the right button) several, one undo
 * step per drag. A cell's brightness
 * shows its velocity. Notes on keys without a pad get rows too (nothing is hidden).
 *
 * The x axis is the piano roll's timeline (a `TimelineViewStore`): the same smooth wheel /
 * trackpad zoom and pan, middle-button pan, the shared ruler (click or drag it to set the play
 * position) and the playhead, on the clip's content axis. Past the clip's end the grid goes on,
 * dimmed and inert (the pattern repeating when the clip loops).
 */

import clsx from "clsx";
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import { useShallow } from "zustand/react/shallow";
import type { Beats, Clip, Command, Note } from "@/generated";
import { useT } from "@/i18n";
import { isDrumRack, useDrumFocus } from "@/features/drum-rack";
import { useGestureSender } from "@/features/devices/gesture";
import { useDeviceWindows } from "@/features/devices/window";
import { Headphones } from "lucide-react";
import { reverseNotesCommand } from "@/features/clip-editing/clipEditing";
import { GrooveControls, grooveQuantizeCommand, useGrooveSettings } from "@/features/groove";
import { Button, IconButton, Select } from "@/kit";
import { useClip, useEditedClipId, useProjectStore } from "@/state";
import { notesOfClip } from "@/state/selectors";
import { cmd, newId, useTransport } from "@/transport";
import { border, radius, size, space, useThemeColor } from "@/theme";
import { parseColor } from "@/features/arrangement/gl/palette";
import { beatRects, BeatRenderer } from "./beatCanvas";
import { createTimelineViewStore, PlayheadLine, Ruler, useMiddleButtonPan, useTempoMap, useTimelineView, useTimelineWheel, useViewport } from "@/timeline";
import { useBeatView } from "./beatView";
import { EditorSwitch } from "./EditorSwitch";
import { clipTempoMap, contentEnd, contentFloor, contentToSong, songToContent } from "./clipTime";
import { pitchName } from "./geometry";
import { playPreview, usePreviewSetting } from "./notePreview";

/** The step sizes (the toolbar's grid; "3" makes them triplets). */
const STEP_OPTIONS: ReadonlyArray<{ label: string; beats: Beats }> = [
  { label: "1/8", beats: 0.5 },
  { label: "1/16", beats: 0.25 },
  { label: "1/32", beats: 0.125 },
];
const DEFAULT_STEP_INDEX = 1;
const BEATS_PER_BAR = 4;
/** Velocity of a painted hit. */
const DEFAULT_VELOCITY = 0.8;
const EPS = 1e-6;
/** Initial zoom (px per beat): a 16th step about as wide as a row is tall. */
const DEFAULT_PX_PER_BEAT = 88;
/** A step's width (px) at the zoom limits: zoomed out, steps stay clickable (and few enough
 *  to render: every visible step is an element). */
const MIN_STEP_PX = 6;
const MAX_STEP_PX = 96;
/** Safety net: never more steps per row than this (a very wide view). */
const MAX_VISIBLE_STEPS = 600;
/** The pad-name column's width (px) limits, and where a resized width is kept. */
const MIN_NAME = 60;
const MAX_NAME = 360;
const NAME_KEY = "eth-beats-name-width";

function savedNameWidth(): number | null {
  try {
    const v = Number(localStorage.getItem(NAME_KEY));
    return v >= MIN_NAME && v <= MAX_NAME ? v : null;
  } catch {
    return null;
  }
}

interface BeatRow {
  note: number;
  name: string;
  /** Not a pad of the rack (notes on that key). */
  stray: boolean;
}

/** The Drum Rack on `clip`'s track (its own chain, not in a rack chain or pad), if any. */
export function useClipDrumRack(clip: Clip | null | undefined): string | null {
  return useProjectStore((s) => {
    const p = s.project;
    if (!p || !clip) return null;
    for (const d of Object.values(p.devices)) {
      if (d.track === clip.track && d.pad === null && !d.chain && isDrumRack(d)) return d.id;
    }
    return null;
  });
}

export function BeatGrid({ clip, rack }: { clip: Clip; rack: string }) {
  const t = useT();
  const sender = useGestureSender();
  const pads = useProjectStore(useShallow((s) => (s.project ? Object.values(s.project.drum_pads).filter((p) => p.rack === rack) : [])));
  const notes = useProjectStore(useShallow((s) => (s.project ? notesOfClip(s.project, clip.id) : [])));

  const transport = useTransport();
  const previewOn = usePreviewSetting((s) => s.enabled);
  const [stepIndex, setStepIndex] = useState(DEFAULT_STEP_INDEX);
  const [triplet, setTriplet] = useState(false);
  /** One step (beats). */
  const step: Beats = STEP_OPTIONS[stepIndex]!.beats * (triplet ? 2 / 3 : 1);

  const regionStart = clip.looping.enabled ? clip.looping.start : clip.offset;
  const floor = contentFloor(clip);
  const steps = Math.max(1, Math.round((contentEnd(clip) - regionStart) / step));

  const rows = useMemo<BeatRow[]>(() => {
    const byNote = new Map<number, BeatRow>();
    for (const p of pads) byNote.set(p.note, { note: p.note, name: p.name.trim() || pitchName(p.note), stray: false });
    for (const n of notes) if (!byNote.has(n.pitch)) byNote.set(n.pitch, { note: n.pitch, name: pitchName(n.pitch), stray: true });
    // The lowest key on top (kick first, like a drum machine).
    return [...byNote.values()].sort((a, b) => a.note - b.note);
  }, [pads, notes]);

  /** The notes starting in step `i` of key `note`. */
  const hits = useMemo(() => {
    const m = new Map<string, Note[]>();
    for (const n of notes) {
      const i = Math.floor((n.start - regionStart) / step + EPS);
      if (i < 0 || i >= steps) continue;
      const k = `${n.pitch}:${i}`;
      m.set(k, [...(m.get(k) ?? []), n]);
    }
    return m;
  }, [notes, regionStart, steps, step]);

  /** The drag in progress: paint ("on") or erase ("off") on one row; the steps done. */
  const drag = useRef<{ note: number; on: boolean; done: Set<number> } | null>(null);

  const apply = (note: number, i: number, on: boolean) => {
    const d = drag.current;
    if (d) {
      if (d.done.has(i)) return;
      d.done.add(i);
    }
    const here = hits.get(`${note}:${i}`) ?? [];
    let command: Command | null = null;
    if (on && here.length === 0) {
      command = cmd("Note", {
        type: "Add",
        clip: clip.id,
        notes: [{ id: newId(), pitch: note, velocity: DEFAULT_VELOCITY, start: regionStart + i * step, duration: step }],
      });
    } else if (!on && here.length > 0) {
      command = cmd("Note", { type: "Remove", ids: here.map((n) => n.id) });
    }
    if (command) void sender.send(command);
    if (on && command) playPreview(transport, clip.track, note, DEFAULT_VELOCITY);
  };

  // ---- The x axis: the piano roll's timeline view, on the clip's content beats ----
  const view = useMemo(
    () =>
      createTimelineViewStore({
        pxPerBeat: DEFAULT_PX_PER_BEAT,
        scrollBeats: regionStart,
        followPlayhead: false,
        // A left-extended clip (leading gap) starts before content 0.
        limits: { minPxPerBeat: MIN_STEP_PX / step, maxPxPerBeat: MAX_STEP_PX / step, minScrollBeats: floor },
      }),
    [], // eslint-disable-line react-hooks/exhaustive-deps
  );
  // The zoom range follows the step size (a step between MIN_STEP_PX and MAX_STEP_PX wide).
  useEffect(() => {
    view.getState().setLimits({ minPxPerBeat: MIN_STEP_PX / step, maxPxPerBeat: MAX_STEP_PX / step, minScrollBeats: floor });
  }, [view, step, floor]);
  const vp = useViewport(view);
  const widthPx = useTimelineView(view, (s) => s.widthPx);
  const songTempo = useTempoMap();
  const tempo = useMemo(() => clipTempoMap(songTempo, clip), [songTempo, clip]);
  const mapping = useMemo(() => (song: Beats) => songToContent(clip, song), [clip]);
  const locate = (beats: Beats) => {
    transport.send(cmd("Transport", { type: "Locate", position: contentToSong(clip, beats) })).catch(() => {});
  };
  const bodyRef = useRef<HTMLDivElement>(null);
  const cornerRef = useRef<HTMLSpanElement>(null);

  // ---- The pad-name column: drag its right edge to resize (double-click: default) ----
  const [nameW, setNameW] = useState<number | null>(savedNameWidth);
  const saveNameW = (w: number | null) => {
    setNameW(w);
    try {
      if (w === null) localStorage.removeItem(NAME_KEY);
      else localStorage.setItem(NAME_KEY, String(w));
    } catch {
      // Storage unavailable: the width lasts for this view only.
    }
  };
  const startResize = (e: ReactPointerEvent<HTMLElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    const x0 = e.clientX;
    const w0 = cornerRef.current?.offsetWidth ?? MIN_NAME;
    let last = w0;
    const move = (ev: PointerEvent) => {
      last = Math.round(Math.min(MAX_NAME, Math.max(MIN_NAME, w0 + ev.clientX - x0)));
      setNameW(last);
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
      saveNameW(last);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
  };
  const nameGrip = (
    <span
      className="eth-beats__name-grip"
      role="separator"
      aria-orientation="vertical"
      aria-label={t("piano_roll.beat.resize_names")}
      onPointerDown={startResize}
      onDoubleClick={() => saveNameW(null)}
    />
  );

  useTimelineWheel(bodyRef, view, { smoothScrollY: true, originPx: nameW ?? parseFloat(size.beatNameWidth) });
  useMiddleButtonPan(bodyRef, view);

  // The visible steps (index from the region start; negative: before it, past `steps`: after
  // the clip's end).
  const cellPx = step * vp.pxPerBeat;
  const firstStep = Math.floor((vp.scrollBeats - regionStart) / step);
  // Before the ruler has measured the view (first frame): the clip's steps.
  const viewBeats = widthPx > 0 ? widthPx / vp.pxPerBeat : steps * step;
  const lastStep = Math.ceil((vp.scrollBeats + viewBeats - regionStart) / step);
  const visible: number[] = [];
  for (let i = Math.max(0, firstStep); i <= Math.min(lastStep, Math.max(0, firstStep) + MAX_VISIBLE_STEPS); i++) visible.push(i);
  const xOf = (i: number) => (regionStart + i * step - vp.scrollBeats) * vp.pxPerBeat;

  // ---- The steps: one canvas (WebGL2) over the rows' step area ----
  const rowH = parseFloat(size.beatRowHeight);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const renderer = useRef<BeatRenderer | null>(null);
  const hover = useRef<{ note: number; i: number } | null>(null);
  const textColor = useThemeColor("text");
  const accentColor = useThemeColor("accent");
  const barColor = useThemeColor("borderStrong");
  /** Where step `i` falls: on a beat (and which), on a bar. */
  const beatOf = (i: number) => Math.floor(i * step + EPS);
  const onBeat = (i: number) => Math.abs(i * step - Math.round(i * step)) < EPS;
  /** Draws with the latest render's state (a ref: hover redraws without a React render). */
  const drawRef = useRef<() => void>(() => {});
  const draw = () => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    renderer.current ??= new BeatRenderer(canvas);
    const velocityAt = (note: number, i: number) => {
      const here = hits.get(`${note}:${i}`);
      return here ? Math.max(...here.map((n) => n.velocity)) : null;
    };
    const rects = beatRects(
      {
        rows: rows.map((r) => r.note),
        rowH,
        visible,
        steps,
        xOf,
        cellW: cellPx,
        hitAt: velocityAt,
        looping: clip.looping.enabled,
        alt: (i) => beatOf(i) % 2 === 1,
        bar: (i) => onBeat(i) && Math.round(i * step) % BEATS_PER_BAR === 0,
        hover: hover.current,
        insetX: parseFloat(border.widthStrong),
        insetY: parseFloat(space.xs),
        radius: parseFloat(radius.sm),
        barWidth: parseFloat(border.width),
      },
      { text: parseColor(textColor), accent: parseColor(accentColor), bar: parseColor(barColor) },
    );
    renderer.current.draw(rects, canvas.clientWidth, canvas.clientHeight);
  };
  const frame = useRef(0);
  const scheduleDraw = () => {
    if (frame.current) return;
    frame.current = requestAnimationFrame(() => {
      frame.current = 0;
      drawRef.current();
    });
  };
  // Every render (zoom, scroll, hits, rows, theme) takes the latest draw and redraws once,
  // before paint.
  useLayoutEffect(() => {
    drawRef.current = draw;
    draw();
  });
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => drawRef.current());
    ro.observe(canvas);
    return () => {
      ro.disconnect();
      cancelAnimationFrame(frame.current);
      frame.current = 0;
      renderer.current?.dispose();
      renderer.current = null;
    };
  }, []);

  /** The step under a pointer: its row's note and step index (in the clip), or null. */
  const hitTest = (clientX: number, clientY: number): { note: number; i: number } | null => {
    const canvas = canvasRef.current;
    if (!canvas) return null;
    const box = canvas.getBoundingClientRect();
    const r = Math.floor((clientY - box.top) / rowH);
    const i = Math.floor(((clientX - box.left) / vp.pxPerBeat + vp.scrollBeats - regionStart) / step + EPS);
    const row = rows[r];
    return row && i >= 0 && i < steps ? { note: row.note, i } : null;
  };
  // The latest hit test and apply, for a drag's window listeners (updated after each commit).
  const hitTestRef = useRef(hitTest);
  const applyRef = useRef(apply);
  useLayoutEffect(() => {
    hitTestRef.current = hitTest;
    applyRef.current = apply;
  });

  /** Left button toggles the pressed step (a drag then paints, or erases if it removed);
   *  the right button removes, also while dragging along the row. */
  const onCanvasDown = (e: ReactPointerEvent<HTMLCanvasElement>) => {
    if (e.button !== 0 && e.button !== 2) return;
    const at = hitTest(e.clientX, e.clientY);
    if (!at) return;
    e.preventDefault();
    const on = e.button === 0 && !hits.get(`${at.note}:${at.i}`)?.length;
    drag.current = { note: at.note, on, done: new Set() };
    sender.begin();
    apply(at.note, at.i, on);
    // Painting: along the pressed row only, one undo step for the drag.
    const move = (ev: PointerEvent) => {
      const d = drag.current;
      const h = hitTestRef.current(ev.clientX, ev.clientY);
      if (d && h && h.note === d.note) applyRef.current(h.note, h.i, d.on);
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
      drag.current = null;
      sender.end();
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
  };
  const onCanvasMove = (e: ReactPointerEvent<HTMLCanvasElement>) => {
    const at = hitTest(e.clientX, e.clientY);
    const prev = hover.current;
    if (prev?.note === at?.note && prev?.i === at?.i) return;
    hover.current = at;
    e.currentTarget.style.cursor = at ? "pointer" : "";
    scheduleDraw();
  };
  const onCanvasLeave = () => {
    if (!hover.current) return;
    hover.current = null;
    scheduleDraw();
  };

  return (
    <div
      className="eth-beats"
      data-testid="beat-grid"
      style={{ ["--_cell" as string]: `${cellPx}px`, ...(nameW !== null && { ["--_name-w" as string]: `${nameW}px` }) }}
    >
      <div className="eth-pr__toolbar">
        <span className="eth-pr__title eth-user-text" title={clip.name}>
          {clip.name || t("piano_roll.clip_fallback")}
        </span>
        <span className="eth-pr__grid-select">
          {t("piano_roll.grid")}
          <Select
            size="sm"
            aria-label={t("piano_roll.beat.step")}
            value={String(stepIndex)}
            options={STEP_OPTIONS.map((o, i) => ({ value: String(i), label: o.label }))}
            onChange={(v) => setStepIndex(Number(v))}
          />
        </span>
        <Button size="sm" active={triplet} onClick={() => setTriplet((t) => !t)} title={t("piano_roll.beat.triplet_steps")}>
          3
        </Button>
        <IconButton
          size="sm"
          tone="ghost"
          className="eth-pr__preview"
          label={previewOn ? t("piano_roll.beat.preview_on") : t("piano_roll.beat.preview_off")}
          active={previewOn}
          icon={<Headphones />}
          onClick={() => usePreviewSetting.getState().setEnabled(!previewOn)}
        />
        <Button
          size="sm"
          disabled={notes.length === 0}
          onClick={() => void sender.send(grooveQuantizeCommand(clip.id, [], useGrooveSettings.getState().quantize, step))}
          title={t("piano_roll.beat.quantize_title")}
        >
          {t("piano_roll.quantize")}
        </Button>
        <Button
          size="sm"
          disabled={notes.length === 0}
          onClick={() => {
            const c = reverseNotesCommand(notes);
            if (c) void sender.send(c);
          }}
          title={t("piano_roll.beat.reverse_title")}
        >
          {t("piano_roll.reverse")}
        </Button>
        <GrooveControls clip={clip.id} selected={[]} rollStep={step} />
        <EditorSwitch track={clip.track} />
      </div>
      <div className="eth-beats__stage">
        <div ref={bodyRef} className="eth-beats__body" role="grid" aria-label={t("piano_roll.beat.steps")}>
          <div className="eth-beats__head">
            <span ref={cornerRef} className="eth-beats__corner">
              {nameGrip}
            </span>
            <div className="eth-beats__ruler">
              <Ruler view={view} showLoop={false} tempo={tempo} playheadMapping={mapping} onLocate={locate} height={parseFloat(size.beatRulerHeight)} />
              {/* The clip's loop (as in the piano roll). */}
              <div className="eth-beats__loopbar" data-testid="beat-loopbar">
                {clip.looping.enabled && (
                  <div
                    className="eth-pr__loop"
                    data-testid="beat-loop"
                    style={{ left: (clip.looping.start - vp.scrollBeats) * vp.pxPerBeat, width: (clip.looping.end - clip.looping.start) * vp.pxPerBeat }}
                    title={t("piano_roll.clip_loop")}
                  />
                )}
                {clip.looping.enabled && (
                  <div className="eth-pr__start" style={{ left: (clip.offset - vp.scrollBeats) * vp.pxPerBeat }} title={t("piano_roll.clip_start")} />
                )}
              </div>
            </div>
          </div>
          <div className="eth-beats__rows">
            {/* Every step of every row: one canvas over the rows' step area. */}
            <canvas
              ref={canvasRef}
              className="eth-beats__canvas"
              data-testid="beat-canvas"
              style={{ height: rows.length * rowH }}
              onPointerDown={onCanvasDown}
              onPointerMove={onCanvasMove}
              onPointerLeave={onCanvasLeave}
              // Right click removes (no menu over the steps).
              onContextMenu={(e) => e.preventDefault()}
            />
            {rows.map((r) => (
              <div key={r.note} className={clsx("eth-beats__row", r.stray && "eth-beats__row--stray")} role="row">
                <span
                  className={clsx("eth-beats__name", "eth-user-text", !r.stray && "eth-beats__name--pad")}
                  role="rowheader"
                  aria-label={r.name}
                  title={r.stray ? t("piano_roll.beat.row_stray", { name: r.name }) : t("piano_roll.beat.row_pad", { name: r.name, note: pitchName(r.note) })}
                  onClick={(e) => {
                    // A pad's row opens the Drum Rack on that pad (not the resize edge).
                    if (r.stray || (e.target as HTMLElement).closest(".eth-beats__name-grip")) return;
                    useDrumFocus.getState().focusPad(rack, r.note);
                    useDeviceWindows.getState().setOpen(rack, true);
                  }}
                >
                  {/* The pad's note as a hint before its name (an unnamed pad's name is the note). */}
                  <span className="eth-beats__note">{r.name !== pitchName(r.note) ? pitchName(r.note) : ""}</span>
                  {r.name}
                  {nameGrip}
                </span>
                <span className="eth-beats__cells" />
              </div>
            ))}
          </div>
          {/* The name column runs to the bottom, below the last pad. */}
          <div className="eth-beats__row eth-beats__row--fill">
            <span className="eth-beats__name" aria-hidden>
              {nameGrip}
            </span>
            {rows.length === 0 && (
              <span className="eth-beats__hint">
                <Button size="sm" onClick={() => useDeviceWindows.getState().setOpen(rack, true)} title={t("piano_roll.beat.add_drums_title")}>
                  {t("piano_roll.beat.add_drums")}
                </Button>
              </span>
            )}
          </div>
        </div>
        {/* The playhead over the steps (the ruler draws its own). */}
        <div className="eth-beats__playhead">
          <PlayheadLine view={view} mapping={mapping} />
        </div>
      </div>
    </div>
  );
}

/** The editor tab's name: "Beat editor" while the open clip shows the step sequencer. */
export function useClipEditorLabel(): string {
  const clip = useClip(useEditedClipId());
  const rack = useClipDrumRack(clip);
  const pianoRoll = useBeatView((s) => (clip ? s.pianoRoll.has(clip.track) : false));
  const t = useT();
  return rack && clip?.content.type === "Midi" && !pianoRoll ? t("piano_roll.tab.beat_editor") : t("piano_roll.tab.piano_roll");
}
