import clsx from "clsx";
import { memo, useEffect, useLayoutEffect, useReducer, useRef, type RefObject } from "react";
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
import type { ClipBounds } from "./editMath";
import { peakLevel, TILE_PEAKS } from "./peaks";

const NOTE_COLOR = "rgba(0, 0, 0, 0.8)";
const WAVE_COLOR = "rgba(0, 0, 0, 0.75)";
const SEAM_COLOR = "rgba(0, 0, 0, 0.35)";

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
  const body: BodyProps = { clip, bounds, from, to, pxWidth: (to - from) * vp.pxPerBeat };

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
      onContextMenu={ghost ? undefined : (e) => openContextMenu(e, clipMenu(ctx.transport, clip))}
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
            ⟳
          </span>
        )}
        {clip.name}
      </div>
      {to > from && (
        <div className="eth-clip__body" style={{ left: (from - bounds.start) * vp.pxPerBeat, width: body.pxWidth }}>
          {clip.content.type === "Midi" ? <MidiPreview {...body} /> : <AudioWaveform {...body} tempo={tempo} />}
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
}

/** Size a canvas to `width` × its css height (device-pixel aware) and redraw it on every render. */
function useCanvasDraw(
  ref: RefObject<HTMLCanvasElement | null>,
  width: number,
  from: Beats,
  to: Beats,
  draw: (ctx: CanvasRenderingContext2D, area: DrawArea) => void,
) {
  useLayoutEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const height = canvas.clientHeight || 30;
    const dpr = window.devicePixelRatio || 1;
    const w = Math.max(1, Math.round(width));
    canvas.width = Math.round(w * dpr);
    canvas.height = Math.round(height * dpr);
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, height);
    draw(ctx, { from, to, width: w, height });
  });
}

/** Mini note preview of a MIDI clip. */
function MidiPreview({ clip, bounds, from, to, pxWidth }: BodyProps) {
  const notes = useNotesOfClip(clip.id);
  const ref = useRef<HTMLCanvasElement>(null);
  useCanvasDraw(ref, pxWidth, from, to, (ctx, area) => {
    const shaped = { ...clip, length: bounds.length, offset: bounds.offset };
    drawNotes(ctx, area, noteRects(shaped, bounds.start, notes, from, to), pitchRange(notes), NOTE_COLOR);
    drawLoopSeams(ctx, area, shaped, bounds.start);
  });
  return <canvas ref={ref} className="eth-clip__canvas" data-testid="clip-notes" data-notes={notes.length} />;
}

/** Waveform of an audio clip from engine peaks (`Media::GetPeaks`, cached in tiles). */
function AudioWaveform({ clip, bounds, from, to, pxWidth, tempo }: BodyProps & { tempo: TempoMap }) {
  const { peaks, transport } = useArrangement();
  const ref = useRef<HTMLCanvasElement>(null);
  const mediaId = clip.content.type === "Audio" ? clip.content.media : null;
  const media: MediaRef | undefined = useProjectStore((s) => (mediaId ? s.project?.media[mediaId] : undefined));
  const markers: WarpMarker[] = useProjectStore(
    useShallow((s) => (s.project ? warpMarkersOfClip(s.project, clip.id) : [])),
  );
  const [, redraw] = useReducer((x: number) => x + 1, 0);
  // Tiles arrive asynchronously: redraw when they do (or when peaks are invalidated).
  useEffect(() => peaks.subscribe(redraw), [peaks]);

  useCanvasDraw(ref, pxWidth, from, to, (ctx, area) => {
    if (!media || clip.content.type !== "Audio") return;
    const toSeconds = clipSourceMapper(clip.content, markers, tempo.bpmAt(bounds.start), bounds.offset, transport.kind);
    const beatsPerPx = (to - from) / Math.max(1, area.width);
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
        level,
        tile: (i) => (i * TILE_PEAKS * level < media.frames ? peaks.tile(media.id, level, i) : null),
      },
      WAVE_COLOR,
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
