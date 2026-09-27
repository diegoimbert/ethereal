/**
 * Warp editor: the detail tab for the audio clip open in the detail editor
 * (`useEditedClipId()`, set by double-clicking a clip in the arrangement).
 *
 * - Toolbar: Warp on/off, mode (Repitch | Complex), transpose (semitones), source tempo.
 *   In the browser build the engine has no stretcher: Complex plays as Repitch, and the
 *   toolbar says so.
 * - Waveform on the clip's content timeline (beats), drawn exactly as the engine maps it
 *   (`clipSourceMapper`), with the warp markers on top.
 * - Markers (warp on): double-click the waveform to add one at that beat (pinned to the
 *   source time currently playing there); drag a marker to move its beat (the audio
 *   stretches; snapped to 1/16, Alt = free); double-click a marker or select it and press
 *   Delete/Backspace to remove it. Every drag (marker or transpose knob) is one undo gesture.
 */

import { useEffect, useMemo, useReducer, useRef, useState, type KeyboardEvent, type PointerEvent, type RefObject } from "react";
import { useShallow } from "zustand/react/shallow";
import type { AudioContent, Beats, Clip, MediaRef, WarpMarker, WarpMarkerId, WarpMode } from "@/generated";
import { Knob, Select, useThemeColor, type SelectOption } from "@/kit";
import { size } from "@/theme";
import { useClip, useEditedClipId, useProjectStore, warpMarkersOfClip } from "@/state";
import { useTempoMap } from "@/timeline";
import { cmd, newId, useTransport } from "@/transport";
import { PeakCache, peakLevel, peakRange, TILE_PEAKS } from "@/features/arrangement/peaks";
import { openGesture, sendOne, startDrag } from "./drag";
import { beatAtSource, clipSourceMapper, compileWarp, complexStretches, sourceSecondsAt } from "./warpMap";
import "./warp.css";

/** Marker snap grid (beats) unless Alt is held. */
export const MARKER_SNAP: Beats = 0.25;
/** Closest two markers may get (beats). */
const MIN_GAP: Beats = 1 / 64;
const WARP_MODES: ReadonlyArray<SelectOption<WarpMode>> = [
  { value: "Repitch", label: "Repitch" },
  { value: "Complex", label: "Complex" },
];
const TRANSPOSE_RANGE = 48;
const HEIGHT = 140;
const RULER = 16;
const FALLBACK_WIDTH = 800;

/** Prop-less warp editor mounted by the app shell. */
export function WarpEditor() {
  const id = useEditedClipId();
  const clip = useClip(id);
  if (!clip) return <Empty text="Double-click an audio clip to edit its warp markers." />;
  if (clip.content.type !== "Audio") return <Empty text="Warp applies to audio clips. This is a MIDI clip." />;
  return <WarpClipEditor key={clip.id} clip={clip} content={clip.content} />;
}

function Empty({ text }: { text: string }) {
  return (
    <div className="eth-warp eth-warp--empty" data-feature="warp" data-testid="warp-empty">
      {text}
    </div>
  );
}

function useElementWidth(ref: RefObject<HTMLElement | null>): number {
  const [w, setW] = useState(0);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    setW(el.clientWidth);
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => setW(el.clientWidth));
    ro.observe(el);
    return () => ro.disconnect();
  }, [ref]);
  return w > 0 ? w : FALLBACK_WIDTH;
}

const snap = (b: Beats, free: boolean) => (free ? b : Math.round(b / MARKER_SNAP) * MARKER_SNAP);

function WarpClipEditor({ clip, content }: { clip: Clip; content: AudioContent }) {
  const transport = useTransport();
  const tempo = useTempoMap();
  const media: MediaRef | undefined = useProjectStore((s) => s.project?.media[content.media]);
  const markers: WarpMarker[] = useProjectStore(useShallow((s) => (s.project ? warpMarkersOfClip(s.project, clip.id) : [])));
  const [selected, setSelected] = useState<WarpMarkerId | null>(null);
  const boxRef = useRef<HTMLDivElement>(null);
  const width = useElementWidth(boxRef);

  const warp = content.warp;
  const refBpm = tempo.bpmAt(clip.start);
  const pins = useMemo(() => compileWarp(warp, markers), [warp, markers]);
  const toSeconds = useMemo(
    () => clipSourceMapper(content, markers, refBpm, clip.offset, transport.kind),
    [content, markers, refBpm, clip.offset, transport.kind],
  );
  const mediaSeconds = media ? media.frames / Math.max(1, media.sample_rate) : 0;
  const contentEnd = clip.looping.enabled ? Math.max(clip.looping.end, clip.offset + clip.length) : clip.offset + clip.length;
  const mediaEnd = mediaSeconds > 0 ? beatAtSource(toSeconds, mediaSeconds) : 0;
  const span = Math.max(1, contentEnd, mediaEnd) * 1.05;
  const pxPerBeat = width / span;
  const xOf = (b: Beats) => b * pxPerBeat;

  const setWarp = (patch: Partial<typeof warp>) =>
    sendOne(transport, cmd("Warp", { type: "SetWarp", clip: clip.id, warp: { ...warp, ...patch } }));

  const addMarkerAt = (clientX: number, free: boolean) => {
    if (!warp.enabled) return;
    const rect = boxRef.current?.getBoundingClientRect();
    const beat = Math.max(0, snap((clientX - (rect?.left ?? 0)) / pxPerBeat, free));
    if (markers.some((m) => Math.abs(m.beat - beat) < MIN_GAP)) return;
    const id = newId();
    setSelected(id);
    sendOne(transport, cmd("Warp", { type: "AddMarker", id, clip: clip.id, beat, source: sourceSecondsAt(pins, refBpm, beat) }));
  };

  const removeMarker = (id: WarpMarkerId) => {
    if (selected === id) setSelected(null);
    sendOne(transport, cmd("Warp", { type: "RemoveMarker", id }));
  };

  const onMarkerDown = (e: PointerEvent, m: WarpMarker) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    e.preventDefault();
    setSelected(m.id);
    boxRef.current?.focus();
    const sorted = [...markers].sort((a, b) => a.beat - b.beat);
    const i = sorted.findIndex((x) => x.id === m.id);
    const lo = i > 0 ? sorted[i - 1]!.beat + MIN_GAP : -Infinity;
    const hi = i < sorted.length - 1 ? sorted[i + 1]!.beat - MIN_GAP : Infinity;
    startDrag(transport, e, (dx, ev) => {
      if (dx === 0) return null;
      const beat = Math.min(hi, Math.max(lo, snap(m.beat + dx / pxPerBeat, ev.altKey)));
      return cmd("Warp", { type: "MoveMarker", id: m.id, beat, source: m.source });
    });
  };

  const onKeyDown = (e: KeyboardEvent) => {
    if ((e.key === "Delete" || e.key === "Backspace") && selected) {
      e.preventDefault();
      removeMarker(selected);
    }
  };

  const webFallback = warp.enabled && warp.mode === "Complex" && !complexStretches(transport.kind);

  return (
    <div className="eth-warp" data-feature="warp" data-testid="warp-editor" data-clip-id={clip.id}>
      <div className="eth-warp__toolbar">
        <span className="eth-warp__title" title={clip.name}>
          {clip.name}
        </span>
        <label className="eth-warp__field">
          <input
            type="checkbox"
            aria-label="Warp"
            checked={warp.enabled}
            onChange={(e) => setWarp({ enabled: e.target.checked })}
          />
          Warp
        </label>
        <span className="eth-warp__field">
          Mode
          <Select<WarpMode>
            size="sm"
            aria-label="Warp mode"
            value={warp.mode}
            disabled={!warp.enabled}
            options={WARP_MODES}
            onChange={(mode) => setWarp({ mode })}
          />
        </span>
        {webFallback && (
          <span className="eth-warp__note" data-testid="warp-web-fallback">
            Complex plays as Repitch in the browser (no time-stretch engine).
          </span>
        )}
        <TransposeControl clip={clip} semitones={content.transpose} />
        <span className="eth-warp__info" data-testid="warp-source-bpm">
          {warp.source_bpm ? `Source ${warp.source_bpm.toFixed(2)} BPM` : "Source tempo unknown"}
        </span>
        <span className="eth-warp__info" data-testid="warp-marker-count">
          {warp.enabled ? `${markers.length} marker${markers.length === 1 ? "" : "s"}` : "Unwarped"}
        </span>
      </div>
      <div
        className="eth-warp__view"
        ref={boxRef}
        tabIndex={0}
        style={{ height: HEIGHT }}
        data-testid="warp-view"
        data-px-per-beat={pxPerBeat}
        onKeyDown={onKeyDown}
        onPointerDown={() => setSelected(null)}
        onDoubleClick={(e) => addMarkerAt(e.clientX, e.altKey)}
      >
        {media && (
          <Waveform
            media={media}
            width={width}
            span={span}
            toSeconds={toSeconds}
            playedFrom={clip.offset}
            playedTo={contentEnd}
          />
        )}
        <svg className="eth-warp__overlay" width={width} height={HEIGHT} data-testid="warp-overlay">
          {Array.from({ length: Math.floor(span) + 1 }, (_, b) => (
            <g key={b}>
              <line x1={xOf(b)} x2={xOf(b)} y1={0} y2={b % 4 === 0 ? RULER : RULER / 2} className="eth-warp__tick" />
              {b % 4 === 0 && pxPerBeat * 4 > 24 && (
                <text x={xOf(b) + 2} y={RULER - 4} className="eth-warp__tick-label">
                  {b / 4 + 1}
                </text>
              )}
            </g>
          ))}
          {warp.enabled &&
            markers.map((m) => (
              <g
                key={m.id}
                className={m.id === selected ? "eth-warp__marker eth-warp__marker--selected" : "eth-warp__marker"}
                data-testid="warp-marker"
                data-marker-id={m.id}
                data-beat={m.beat}
                transform={`translate(${xOf(m.beat)}, 0)`}
                onPointerDown={(e) => onMarkerDown(e, m)}
                onDoubleClick={(e) => {
                  e.stopPropagation();
                  removeMarker(m.id);
                }}
              >
                <line x1={0} x2={0} y1={RULER} y2={HEIGHT} />
                <path d={`M -6 0 L 6 0 L 6 ${RULER - 6} L 0 ${RULER} L -6 ${RULER - 6} Z`} />
                <title>{`beat ${m.beat.toFixed(3)} → ${m.source.toFixed(3)} s`}</title>
              </g>
            ))}
        </svg>
      </div>
    </div>
  );
}

function TransposeControl({ clip, semitones }: { clip: Clip; semitones: number }) {
  const transport = useTransport();
  const gesture = useRef<ReturnType<typeof openGesture> | null>(null);
  const set = (st: number) =>
    cmd("Clip", { type: "SetTranspose", id: clip.id, semitones: Math.max(-TRANSPOSE_RANGE, Math.min(TRANSPOSE_RANGE, st)) });
  return (
    <span className="eth-warp__field eth-warp__transpose">
      <Knob
        value={(semitones + TRANSPOSE_RANGE) / (2 * TRANSPOSE_RANGE)}
        bipolar
        size={parseFloat(size.knobSm)}
        label="Transpose"
        valueText={`${semitones > 0 ? "+" : ""}${semitones} st`}
        onChangeStart={() => {
          gesture.current = openGesture(transport);
        }}
        onChange={(v) => {
          const st = Math.round(v * 2 * TRANSPOSE_RANGE - TRANSPOSE_RANGE);
          // Inside a drag: part of its gesture. Keyboard/wheel/reset: one undo step each.
          if (gesture.current) gesture.current.send(set(st));
          else sendOne(transport, set(st));
        }}
        onChangeEnd={() => {
          gesture.current?.end();
          gesture.current = null;
        }}
      />
      <input
        type="number"
        aria-label="Transpose"
        className="eth-warp__number"
        min={-TRANSPOSE_RANGE}
        max={TRANSPOSE_RANGE}
        step={1}
        value={semitones}
        onChange={(e) => {
          const v = Number(e.target.value);
          if (Number.isFinite(v)) sendOne(transport, set(v));
        }}
      />
      st
    </span>
  );
}

function Waveform({
  media,
  width,
  span,
  toSeconds,
  playedFrom,
  playedTo,
}: {
  media: MediaRef;
  width: number;
  span: Beats;
  toSeconds: (c: Beats) => number;
  /** Content range the clip plays (drawn brighter). */
  playedFrom: Beats;
  playedTo: Beats;
}) {
  const transport = useTransport();
  const peaks = useMemo(() => new PeakCache(transport), [transport]);
  const ref = useRef<HTMLCanvasElement>(null);
  const [tick, redraw] = useReducer((x: number) => x + 1, 0);
  useEffect(() => peaks.subscribe(redraw), [peaks]);
  const waveColor = useThemeColor("warpWave");
  const waveDim = useThemeColor("warpWaveDim");

  useEffect(() => {
    const canvas = ref.current;
    const ctx = canvas?.getContext?.("2d");
    if (!canvas || !ctx) return;
    const dpr = window.devicePixelRatio || 1;
    const h = HEIGHT - RULER;
    canvas.width = Math.round(width * dpr);
    canvas.height = Math.round(h * dpr);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, h);
    const bpp = span / Math.max(1, width);
    const sr = media.sample_rate;
    const level = peakLevel(Math.abs(toSeconds(bpp) - toSeconds(0)) * sr);
    const tile = (i: number) => (i * TILE_PEAKS * level < media.frames ? peaks.tile(media.id, level, i) : null);
    const mid = h / 2;
    for (const [color, inside] of [
      [waveDim, false],
      [waveColor, true],
    ] as const) {
      ctx.fillStyle = color;
      ctx.beginPath();
      for (let x = 0; x < width; x++) {
        const c = x * bpp;
        if ((c >= playedFrom && c < playedTo) !== inside) continue;
        const f0 = Math.floor(toSeconds(c) * sr);
        const f1 = Math.ceil(toSeconds(c + bpp) * sr);
        const a = Math.max(0, Math.min(f0, f1));
        const b = Math.min(media.frames, Math.max(f0, f1));
        if (b <= a) continue;
        const p = peakRange(a, b, level, tile);
        if (!p) continue;
        const top = mid - p.max * mid;
        ctx.rect(x, top, 1, Math.max(1, mid - p.min * mid - top));
      }
      ctx.fill();
    }
  }, [media, width, span, toSeconds, playedFrom, playedTo, peaks, tick, waveColor, waveDim]);

  return (
    <canvas
      ref={ref}
      className="eth-warp__canvas"
      style={{ width, height: HEIGHT - RULER, top: RULER }}
      data-testid="warp-waveform"
    />
  );
}
