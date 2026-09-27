/**
 * Live recording overlay of one arrangement lane (`live-record`): a "recording" clip per
 * take growing to the playhead, drawing the peaks received so far with the arrangement's
 * waveform code (`drawWaveform`), and the MIDI notes played (held notes grow). Rendered
 * inside the lane's `LaneLayer` (positions in beats from `origin`, like clips). It only
 * overlays: the user's clips are never restyled. On `Stopped` it freezes over the committed
 * clips (which land exactly where it was) until their waveform is loaded, then disappears.
 */

import clsx from "clsx";
import { useEffect, useLayoutEffect, useRef } from "react";
import type { Beats, Color, Note, TrackId } from "@/generated";
import { drawNotes, drawWaveform, pitchRange, type DrawArea } from "@/features/arrangement/clipDraw";
import { clipInk, colorCss } from "@/features/arrangement/helpers";
import { beatsCss, widthCss } from "@/features/arrangement/laneGeometry";
import { playheadStore } from "@/state";
import { useTheme } from "@/theme";
import { useTempoMap, type TempoMap } from "@/timeline";
import type { EngineTransport } from "@/transport";
import { liveEnd, noteSpans, takeEnd, type LiveNote, type LiveTake, type LivePhase } from "./liveModel";
import { useLiveRecordingFeed, useLiveTrack } from "./liveStore";

/** Longest canvas side in device px. */
const MAX_CANVAS_PX = 16384;
/** How far behind the playhead (seconds) a take's data may lag and still be growing. */
const GROW_SLACK_SECONDS = 0.5;

export interface LiveRecordLaneProps {
  transport: EngineTransport;
  track: TrackId;
  color: Color;
  /** Lane layout origin (beats; see `LaneLayer`). */
  origin: Beats;
  /** Visible timeline range (the canvas only covers it). */
  visible: { start: Beats; end: Beats };
  /** Settled zoom the lane is laid out at. */
  pxPerBeat: number;
}

export function LiveRecordLane(props: LiveRecordLaneProps) {
  useLiveRecordingFeed(props.transport);
  const view = useLiveTrack(props.track);
  const tempo = useTempoMap();
  const [theme] = useTheme();
  if (view.takes.length === 0 && view.notes.length === 0) return null;
  const ink = clipInk(colorCss(props.color), theme);
  const latest = view.takes[view.takes.length - 1];
  return (
    <>
      {view.takes.map((t) => (
        <LiveClip key={`a${t.take}`} {...props} phase={view.phase} tempo={tempo} ink={ink} take={t} growing={t === latest} />
      ))}
      {view.notes.length > 0 && <LiveClip {...props} phase={view.phase} tempo={tempo} ink={ink} notes={view.notes} growing />}
    </>
  );
}

interface LiveClipProps extends LiveRecordLaneProps {
  phase: LivePhase;
  tempo: TempoMap;
  ink: string;
  growing: boolean;
  take?: LiveTake;
  notes?: ReadonlyArray<LiveNote>;
}

/** Timeline extent of a live clip now. */
function extent(p: Pick<LiveClipProps, "take" | "notes" | "tempo" | "phase" | "growing">, playhead: number | null) {
  const growing = p.growing && p.phase === "recording";
  if (p.take) {
    const bpm = p.tempo.bpmAt(p.take.start);
    const start = Math.max(0, p.take.start);
    const end = liveEnd(takeEnd(p.take, bpm), playhead, growing, (GROW_SLACK_SECONDS * bpm) / 60);
    return { start, end, bpm };
  }
  const notes = p.notes ?? [];
  const now = growing && playhead !== null ? playhead : -Infinity;
  const spans = noteSpans(notes, now);
  const start = Math.max(0, Math.min(...spans.map((s) => s.start)));
  const last = Math.max(...spans.map((s) => s.end));
  const bpm = p.tempo.bpmAt(start);
  return { start, end: growing && playhead !== null && playhead >= start ? Math.max(last, playhead) : last, bpm };
}

function LiveClip(p: LiveClipProps) {
  const box = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const props = useRef(p);
  useLayoutEffect(() => {
    props.current = p;
  });

  useEffect(() => {
    let frame = 0;
    const draw = () => {
      frame = 0;
      const q = props.current;
      const el = box.current;
      const cv = canvas.current;
      if (!el || !cv) return;
      const playhead = playheadStore.getPlayhead()?.transport.position ?? null;
      const { start, end, bpm } = extent(q, playhead);
      el.style.left = beatsCss(start - q.origin);
      el.style.width = widthCss(Math.max(0, end - start));
      const from = Math.max(start, q.visible.start);
      const to = Math.min(end, q.visible.end);
      if (!(to > from)) {
        cv.style.display = "none";
        return;
      }
      cv.style.display = "";
      cv.style.left = beatsCss(from - start);
      cv.style.width = beatsCss(to - from);
      const dpr = window.devicePixelRatio || 1;
      const w = Math.max(1, Math.round((to - from) * q.pxPerBeat));
      const h = cv.clientHeight || 30;
      cv.width = Math.min(MAX_CANVAS_PX, Math.round(w * dpr));
      cv.height = Math.round(h * dpr);
      const ctx = cv.getContext("2d");
      if (!ctx) return;
      ctx.setTransform(cv.width / w, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, w, h);
      const area: DrawArea = { from, to, width: w, height: h };
      if (q.take) {
        const t = q.take;
        const len = takeEnd(t, bpm) - t.start;
        drawWaveform(
          ctx,
          area,
          {
            start: t.start,
            clip: { length: len, offset: 0, looping: { enabled: false, start: 0, end: len } },
            sampleRate: t.sampleRate,
            toSeconds: (c) => (c * 60) / bpm,
            frames: t.frames,
            level: t.framesPerPeak,
            tile: (i) => t.tile(i),
          },
          q.ink,
        );
      } else if (q.notes) {
        const now = q.growing && q.phase === "recording" && playhead !== null ? playhead : -Infinity;
        const spans = noteSpans(q.notes, now);
        const rects = spans
          .filter((s) => s.end > from && s.start < to)
          .map((s) => ({ t0: Math.max(s.start, from), t1: Math.min(s.end, to), pitch: s.pitch, velocity: s.velocity, muted: false }));
        drawNotes(ctx, area, rects, pitchRange(q.notes as unknown as Note[]), q.ink);
      }
    };
    const schedule = () => {
      if (!frame) frame = requestAnimationFrame(draw);
    };
    draw();
    const off = playheadStore.subscribePlayhead(schedule);
    return () => {
      off();
      if (frame) cancelAnimationFrame(frame);
    };
  });

  const audio = p.take !== undefined;
  return (
    <div
      ref={box}
      className={clsx(
        "eth-clip",
        audio ? "eth-clip--audio" : "eth-clip--midi",
        "eth-clip--live",
        p.phase === "handoff" && "eth-clip--live-handoff",
      )}
      style={{ ["--eth-clip-color" as string]: colorCss(p.color) }}
      data-testid="live-clip"
      data-live-kind={audio ? "audio" : "midi"}
      data-live-take={p.take?.take}
      data-live-peaks={p.take?.count}
      data-live-notes={p.notes?.length}
      data-live-phase={p.phase}
      aria-hidden
    >
      <div className="eth-clip__title">
        <span className="eth-clip__name">Recording</span>
      </div>
      <div className="eth-clip__body" style={{ left: 0, width: "100%" }}>
        <canvas ref={canvas} className="eth-clip__canvas eth-clip__canvas--live" />
      </div>
    </div>
  );
}
