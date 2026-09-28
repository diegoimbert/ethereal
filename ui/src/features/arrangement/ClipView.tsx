import clsx from "clsx";
import { memo, useEffect, useLayoutEffect, useMemo, useReducer, useRef, type RefObject } from "react";
import { Repeat } from "lucide-react";
import { useShallow } from "zustand/react/shallow";
import type { Beats, Clip, Color, MediaRef, WarpMarker } from "@/generated";
import { clipInk, colorCss } from "./helpers";
import { useTheme } from "@/theme";
import { useClipEditors } from "@/features/collab/presence/editors";
import { useEditorStore, useNotesOfClip, useProjectStore, warpMarkersOfClip } from "@/state";
import { useIsSelected, type TempoMap, type TimelineViewport } from "@/timeline";
import { openContextMenu, useThemeColor } from "@/kit";
import { withFreezeClipEntries } from "@/features/freeze";
import { MissingClipBadge, useMediaMissing, withMediaRefClipEntries } from "@/features/media-refs";
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
import { beatsCss, widthCss } from "./laneGeometry";
import { arrangementView } from "./uiStore";

/** Narrowest a clip is drawn, so it stays grabbable (title bar) when zoomed far out. */
const CLIP_MIN_PX = 8;


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
  // Collab: peers with this clip open in their piano roll (ring in the first one's color).
  const editors = useClipEditors(ghost ? null : clip.id);
  const color = colorCss(clip.color ?? trackColor);
  // Positioned in beats at the live zoom (--ppb, see laneGeometry.ts), never narrower than
  // CLIP_MIN_PX so even tiny clips can be grabbed.
  const left = beatsCss(bounds.start - vp.scrollBeats);
  const width = widthCss(bounds.length, CLIP_MIN_PX);
  const from = Math.max(bounds.start, visible.start);
  const to = Math.min(bounds.start + bounds.length, visible.end);
  const [theme] = useTheme();
  const body: BodyProps = { clip, bounds, from, to, pxWidth: (to - from) * vp.pxPerBeat, ink: clipInk(color, theme) };
  // media-references: the clip's sample can't be found (plays silence until relinked).
  const sample = clip.content.type === "Audio" ? clip.content.media : null;
  const missing = useMediaMissing(ghost ? null : sample);

  return (
    <div
      className={clsx(
        "eth-clip",
        clip.content.type === "Midi" ? "eth-clip--midi" : "eth-clip--audio",
        selected && !ghost && "eth-clip--selected",
        clip.muted && "eth-clip--muted",
        dragging && "eth-clip--dragging",
        ghost && "eth-clip--ghost",
        editors.length > 0 && "eth-clip--peer-editing",
        missing && "eth-media-refs--missing",
      )}
      style={{
        left,
        width,
        ["--eth-clip-color" as string]: color,
        ...(editors.length > 0 ? { ["--eth-collab-peer" as string]: editors[0]!.color } : {}),
      }}
      data-clip-id={ghost ? undefined : clip.id}
      data-ghost={ghost ? clip.id : undefined}
      role={ghost ? undefined : "button"}
      aria-label={ghost ? undefined : clip.name || "Clip"}
      aria-pressed={ghost ? undefined : selected}
      onPointerDown={ghost ? undefined : (e) => onClipPointerDown(e, clip, ctx)}
      onContextMenu={ghost ? undefined : (e) => openContextMenu(e, withMediaRefClipEntries(withFreezeClipEntries(withClipEditingEntries(clipMenu(ctx.transport, clip), ctx.transport, clip), ctx.transport, clip), clip))}
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
        {!ghost && <MissingClipBadge media={sample} />}
        {clip.name && <span className="eth-clip__name">{clip.name}</span>}
        {editors.length > 0 && (
          <span
            className="eth-clip__editors"
            data-testid="clip-editors"
            title={`${editors.map((e) => e.name).join(", ")} ${editors.length === 1 ? "is" : "are"} editing this clip`}
          >
            {editors.map((e) => e.name).join(", ")}
          </span>
        )}
      </div>
      {to > from && (
        <div className="eth-clip__body" style={{ left: beatsCss(from - bounds.start), width: widthCss(to - from) }}>
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

/**
 * A clip body canvas that survives scrolling and zooming cheaply. It is drawn for the
 * visible part of the clip plus about one viewport on each side (clamped to the clip),
 * and placed in beats with CSS (`--ppb`, see laneGeometry.ts), so while the view pans or
 * zooms the browser just moves and stretches it. It is redrawn when a render finds the
 * view outside the drawn window, the zoom changed (the lane re-renders once the zoom
 * settles, or when it drifts too far), or `content` changed (a key for what `draw` depends
 * on). The expensive peak rendering happens rarely instead of every frame.
 */
function useCanvasDraw(
  ref: RefObject<HTMLCanvasElement | null>,
  body: BodyProps,
  content: unknown,
  draw: (ctx: CanvasRenderingContext2D, area: DrawArea) => void,
) {
  const drawn = useRef<{ from: Beats; to: Beats; ppb: number; content: unknown; height: number } | null>(null);
  const drawRef = useRef(draw);
  useLayoutEffect(() => {
    drawRef.current = draw;
  });

  const { from, to, bounds } = body;

  useLayoutEffect(() => {
    const canvas = ref.current;
    if (!canvas || !(to > from)) return;
    const view = arrangementView.getState();
    const p = view.pxPerBeat;
    const d = drawn.current;
    const stale =
      !d ||
      d.content !== content ||
      d.ppb !== p ||
      from < d.from - 1e-9 ||
      to > d.to + 1e-9 ||
      (canvas.clientHeight || 30) !== d.height;
    if (stale) {
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
      drawn.current = { from: wFrom, to: wTo, ppb: p, content, height };
      const ctx = canvas.getContext("2d");
      if (ctx) {
        ctx.setTransform(canvas.width / w, 0, 0, dpr, 0, 0);
        ctx.clearRect(0, 0, w, height);
        drawRef.current(ctx, { from: wFrom, to: wTo, width: w, height });
      }
    }
    // Place the drawn window relative to the body (which starts at `from`), in beats.
    const cur = drawn.current!;
    canvas.style.position = "absolute";
    canvas.style.top = "0";
    canvas.style.height = "100%";
    canvas.style.left = beatsCss(cur.from - from);
    canvas.style.width = beatsCss(cur.to - cur.from);
  });
}

/** Mini note preview of a MIDI clip. */
function MidiPreview(body: BodyProps) {
  const { clip, bounds } = body;
  const notes = useNotesOfClip(clip.id);
  const seam = useThemeColor("clipSeam");
  const ref = useRef<HTMLCanvasElement>(null);
  const content = useMemo(
    () => [clip, bounds.start, bounds.length, bounds.offset, notes, body.ink, seam],
    [clip, bounds.start, bounds.length, bounds.offset, notes, body.ink, seam],
  );
  useCanvasDraw(ref, body, content, (ctx, area) => {
    const shaped = { ...clip, length: bounds.length, offset: bounds.offset };
    drawNotes(ctx, area, noteRects(shaped, bounds.start, notes, area.from, area.to), pitchRange(notes), body.ink);
    drawLoopSeams(ctx, area, shaped, bounds.start, seam);
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
  const seam = useThemeColor("clipSeam");
  const [tiles, redraw] = useReducer((x: number) => x + 1, 0);
  // Tiles arrive asynchronously: redraw when they do (or when peaks are invalidated).
  useEffect(() => peaks.subscribe(redraw), [peaks]);

  const content = useMemo(
    () => [clip, bounds.start, bounds.length, bounds.offset, media, markers, tempo, tiles, body.ink, seam],
    [clip, bounds.start, bounds.length, bounds.offset, media, markers, tempo, tiles, body.ink, seam],
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
    drawLoopSeams(ctx, area, shaped, bounds.start, seam);
  });
  return <canvas ref={ref} className="eth-clip__canvas" data-testid="clip-waveform" data-media={mediaId ?? ""} />;
}

/** Thin markers where a looped clip wraps around to its loop start. */
function drawLoopSeams(
  ctx: CanvasRenderingContext2D,
  area: DrawArea,
  clip: Pick<Clip, "length" | "offset" | "looping">,
  start: Beats,
  color: string,
) {
  if (!clip.looping.enabled) return;
  const ppb = area.width / Math.max(1e-9, area.to - area.from);
  ctx.fillStyle = color;
  for (const seg of contentSegments(clip, start, area.from, area.to)) {
    if (seg.t0 <= area.from || seg.c0 !== clip.looping.start) continue;
    ctx.fillRect(Math.round((seg.t0 - area.from) * ppb), 0, 1, area.height);
  }
}
