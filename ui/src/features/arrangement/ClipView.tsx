import clsx from "clsx";
import { memo, useEffect, useLayoutEffect, useMemo, useReducer, useRef, type RefObject } from "react";
import { Repeat } from "lucide-react";
import { useShallow } from "zustand/react/shallow";
import type { Beats, Clip, Color, MediaRef, WarpMarker } from "@/generated";
import { colorCss } from "./helpers";
import { useEditorStore, useNotesOfClip, useProjectStore, warpMarkersOfClip } from "@/state";
import { useIsSelected, type TempoMap, type TimelineViewport } from "@/timeline";
import { openContextMenu } from "@/kit";
import { clipMenu } from "./actions";
import { onClipPointerDown } from "./clipDrag";
import { drawNotes, drawWaveform, noteRects, pitchRange, type DrawArea } from "./clipDraw";
import { contentSegments } from "./clipTime";
import { clipSourceMapper } from "@/features/warp/warpMap";
import { useArrangement } from "./context";
import { ClipFades, ReversedBadge } from "@/features/clip-editing";
import { withClipEditingEntries } from "@/features/clip-editing/clipEditing";
import type { ClipBounds } from "./editMath";
import { peakLevel, TILE_PEAKS } from "./peaks";
import { arrangementView } from "./uiStore";

const SEAM_COLOR = "rgba(0, 0, 0, 0.25)";


export interface ClipViewProps {
  clip: Clip;
  bounds: ClipBounds;
  trackColor: Color;
  vp: TimelineViewport;
  /** Visible timeline range (the canvas only covers the visible part of the clip). */
  visible: { start: Beats; end: Beats };
  tempo: TempoMap;
  dragging?: boolean;
  ghost?: boolean;
}

export const ClipView = memo(function ClipView({ clip, bounds, trackColor, vp, visible, tempo, dragging, ghost }: ClipViewProps) {
  const ctx = useArrangement();
  const selected = useIsSelected("clip", clip.id);
  const color = colorCss(clip.color ?? trackColor);
  const left = (bounds.start - vp.scrollBeats) * vp.pxPerBeat;
  const width = Math.max(2, bounds.length * vp.pxPerBeat);
  const from = Math.max(bounds.start, visible.start);
  const to = Math.min(bounds.start + bounds.length, visible.end);
  const body: BodyProps = { clip, bounds, from, to, pxWidth: (to - from) * vp.pxPerBeat, ink: color };

  return (
    <div
      className={clsx(
        "eth-clip",
        clip.content.type === "Midi" ? "eth-clip--midi" : "eth-clip--audio",
        selected && !ghost && "eth-clip--selected",
        clip.muted && "eth-clip--muted",
        dragging && "eth-clip--dragging",
        ghost && "eth-clip--ghost",
      )}
      style={{ left, width, ["--eth-clip-color" as string]: color }}
      data-clip-id={ghost ? undefined : clip.id}
      data-ghost={ghost ? clip.id : undefined}
      role={ghost ? undefined : "button"}
      aria-label={ghost ? undefined : clip.name || "Clip"}
      aria-pressed={ghost ? undefined : selected}
      onPointerDown={ghost ? undefined : (e) => onClipPointerDown(e, clip, ctx)}
      onContextMenu={ghost ? undefined : (e) => openContextMenu(e, withClipEditingEntries(clipMenu(ctx.transport, clip), ctx.transport, clip))}
      onDoubleClick={
        ghost
          ? undefined
          : (e) => {
              e.stopPropagation();
              useEditorStore.getState().openClip(clip.id);
            }
      }
    >
      <div className="eth-clip__title">
        {clip.looping.enabled && (
          <span className="eth-clip__loop" title="Looping">
            <Repeat />
          </span>
        )}
        {clip.content.type === "Audio" && clip.content.reversed && <ReversedBadge />}
        {clip.name && <span className="eth-clip__name">{clip.name}</span>}
      </div>
      {to > from && (
        <div className="eth-clip__body" style={{ left: (from - bounds.start) * vp.pxPerBeat, width: body.pxWidth }}>
          {clip.content.type === "Midi" ? <MidiPreview {...body} /> : <AudioWaveform {...body} tempo={tempo} />}
        </div>
      )}
      {!ghost && clip.content.type === "Audio" && (
        <div className="eth-clip__body" style={{ left: 0, width }}>
          <ClipFades clip={clip} length={bounds.length} pxPerBeat={vp.pxPerBeat} transport={ctx.transport} />
        </div>
      )}
      {!ghost && (
        <>
          <div className="eth-clip__handle eth-clip__handle--start" data-handle="resize-start" />
          <div className="eth-clip__handle eth-clip__handle--end" data-handle="resize-end" />
        </>
      )}
    </div>
  );
});

interface BodyProps {
  clip: Clip;
  bounds: ClipBounds;
  /** Drawn timeline range and its width in css px. */
  from: Beats;
  to: Beats;
  pxWidth: number;
  /** Color of the notes / waveform: the clip's own (bright) color, like its border and header. */
  ink: string;
}

/** Longest canvas side in device px (browsers cap canvases around 16k–32k). */
const MAX_CANVAS_PX = 8192;
/** Redraw at once when the zoom drifts this far from the drawn one (stretching blurs). */
const MAX_STRETCH = 1.5;
/** After the zoom stops changing, redraw sharp at the final zoom. */
const SETTLE_MS = 120;

/**
 * A clip body canvas that survives scrolling and zooming cheaply. It is drawn for the
 * visible part of the clip plus about one viewport on each side (clamped to the clip),
 * then only repositioned while the view pans, and stretched by CSS while it zooms. It is
 * redrawn when the view leaves the drawn window, when the zoom drifts past
 * `MAX_STRETCH` (plus once, sharp, when the zoom settles), or when `content` changes
 * (a key for what `draw` depends on). This keeps animated pan/zoom smooth with many
 * waveforms: the expensive peak rendering happens rarely instead of every frame.
 */
function useCanvasDraw(
  ref: RefObject<HTMLCanvasElement | null>,
  body: BodyProps,
  content: unknown,
  draw: (ctx: CanvasRenderingContext2D, area: DrawArea) => void,
) {
  const drawn = useRef<{ from: Beats; to: Beats; ppb: number; content: unknown; height: number } | null>(null);
  const settle = useRef<ReturnType<typeof setTimeout> | null>(null);
  const drawRef = useRef(draw);
  useLayoutEffect(() => {
    drawRef.current = draw;
  });
  useEffect(() => () => {
    if (settle.current) clearTimeout(settle.current);
  }, []);

  const { from, to, bounds } = body;
  const ppb = body.pxWidth / Math.max(1e-9, to - from);

  useLayoutEffect(() => {
    const canvas = ref.current;
    if (!canvas || !(to > from)) return;

    const paint = () => {
      const view = arrangementView.getState();
      const p = view.pxPerBeat;
      const clipEnd = bounds.start + bounds.length;
      const dpr = window.devicePixelRatio || 1;
      const margin = Math.max(to - from, (view.widthPx || 1000) / p);
      let wFrom = Math.max(bounds.start, from - margin);
      let wTo = Math.min(clipEnd, to + margin);
      // Keep the canvas within the browser's size limit (shrink the margins first).
      const maxBeats = MAX_CANVAS_PX / (p * dpr);
      if (wTo - wFrom > maxBeats) {
        const extra = (maxBeats - (to - from)) / 2;
        wFrom = Math.max(bounds.start, from - Math.max(0, extra));
        wTo = Math.min(clipEnd, wFrom + maxBeats);
      }
      const height = canvas.clientHeight || 30;
      const w = Math.max(1, Math.round((wTo - wFrom) * p));
      canvas.width = Math.min(MAX_CANVAS_PX, Math.round(w * dpr));
      canvas.height = Math.round(height * dpr);
      const ctx = canvas.getContext("2d");
      drawn.current = { from: wFrom, to: wTo, ppb: p, content, height };
      if (ctx) {
        ctx.setTransform(canvas.width / w, 0, 0, dpr, 0, 0);
        ctx.clearRect(0, 0, w, height);
        drawRef.current(ctx, { from: wFrom, to: wTo, width: w, height });
      }
    };

    const d = drawn.current;
    const stretch = d ? Math.max(ppb / d.ppb, d.ppb / ppb) : Infinity;
    const stale =
      !d ||
      d.content !== content ||
      from < d.from - 1e-9 ||
      to > d.to + 1e-9 ||
      stretch > MAX_STRETCH ||
      (canvas.clientHeight || 30) !== d.height;
    if (stale) paint();
    else if (stretch > 1 + 1e-6) {
      // Zooming: stretch now, redraw sharp once the zoom stops changing.
      if (settle.current) clearTimeout(settle.current);
      settle.current = setTimeout(() => {
        settle.current = null;
        paint();
        place();
      }, SETTLE_MS);
    }
    place();

    function place() {
      const cur = drawn.current;
      if (!canvas || !cur) return;
      // Position the drawn window relative to the body (which starts at `from`).
      canvas.style.position = "absolute";
      canvas.style.top = "0";
      canvas.style.height = "100%";
      canvas.style.left = `${(cur.from - from) * ppb}px`;
      canvas.style.width = `${(cur.to - cur.from) * ppb}px`;
    }
  });
}

/** Mini note preview of a MIDI clip. */
function MidiPreview(body: BodyProps) {
  const { clip, bounds } = body;
  const notes = useNotesOfClip(clip.id);
  const ref = useRef<HTMLCanvasElement>(null);
  const content = useMemo(
    () => [clip, bounds.start, bounds.length, bounds.offset, notes, body.ink],
    [clip, bounds.start, bounds.length, bounds.offset, notes, body.ink],
  );
  useCanvasDraw(ref, body, content, (ctx, area) => {
    const shaped = { ...clip, length: bounds.length, offset: bounds.offset };
    drawNotes(ctx, area, noteRects(shaped, bounds.start, notes, area.from, area.to), pitchRange(notes), body.ink);
    drawLoopSeams(ctx, area, shaped, bounds.start);
  });
  return <canvas ref={ref} className="eth-clip__canvas" data-testid="clip-notes" data-notes={notes.length} />;
}

/** Waveform of an audio clip from engine peaks (`Media::GetPeaks`, cached in tiles). */
function AudioWaveform({ tempo, ...body }: BodyProps & { tempo: TempoMap }) {
  const { clip, bounds } = body;
  const { peaks, transport } = useArrangement();
  const ref = useRef<HTMLCanvasElement>(null);
  const mediaId = clip.content.type === "Audio" ? clip.content.media : null;
  const media: MediaRef | undefined = useProjectStore((s) => (mediaId ? s.project?.media[mediaId] : undefined));
  const markers: WarpMarker[] = useProjectStore(
    useShallow((s) => (s.project ? warpMarkersOfClip(s.project, clip.id) : [])),
  );
  const [tiles, redraw] = useReducer((x: number) => x + 1, 0);
  // Tiles arrive asynchronously: redraw when they do (or when peaks are invalidated).
  useEffect(() => peaks.subscribe(redraw), [peaks]);

  const content = useMemo(
    () => [clip, bounds.start, bounds.length, bounds.offset, media, markers, tempo, tiles, body.ink],
    [clip, bounds.start, bounds.length, bounds.offset, media, markers, tempo, tiles, body.ink],
  );
  useCanvasDraw(ref, body, content, (ctx, area) => {
    if (!media || clip.content.type !== "Audio") return;
    const toSeconds = clipSourceMapper(clip.content, markers, tempo.bpmAt(bounds.start), bounds.offset, transport.kind);
    const beatsPerPx = (area.to - area.from) / Math.max(1, area.width);
    const level = peakLevel(Math.abs(toSeconds(beatsPerPx) - toSeconds(0)) * media.sample_rate);
    const shaped = { length: bounds.length, offset: bounds.offset, looping: clip.looping };
    drawWaveform(
      ctx,
      area,
      {
        start: bounds.start,
        clip: shaped,
        sampleRate: media.sample_rate,
        toSeconds,
        frames: media.frames,
        reversed: clip.content.reversed,
        level,
        tile: (i) => (i * TILE_PEAKS * level < media.frames ? peaks.tile(media.id, level, i) : null),
      },
      body.ink,
    );
    drawLoopSeams(ctx, area, shaped, bounds.start);
  });
  return <canvas ref={ref} className="eth-clip__canvas" data-testid="clip-waveform" data-media={mediaId ?? ""} />;
}

/** Thin markers where a looped clip wraps around to its loop start. */
function drawLoopSeams(
  ctx: CanvasRenderingContext2D,
  area: DrawArea,
  clip: Pick<Clip, "length" | "offset" | "looping">,
  start: Beats,
) {
  if (!clip.looping.enabled) return;
  const ppb = area.width / Math.max(1e-9, area.to - area.from);
  ctx.fillStyle = SEAM_COLOR;
  for (const seg of contentSegments(clip, start, area.from, area.to)) {
    if (seg.t0 <= area.from || seg.c0 !== clip.looping.start) continue;
    ctx.fillRect(Math.round((seg.t0 - area.from) * ppb), 0, 1, area.height);
  }
}
