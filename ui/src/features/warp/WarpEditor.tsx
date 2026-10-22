/**
 * Warp editor: the detail tab for the audio clip open in the detail editor
 * (`useEditedClipId()`, set by double-clicking a clip in the arrangement).
 *
 * - Toolbar: Warp on/off, mode (Repitch | Complex), transpose (semitones), source tempo.
 *   Complex keeps pitch in every build (the browser runs a pure-Rust Signalsmith Stretch);
 *   should an engine lack a stretcher, the toolbar says Complex plays as Repitch.
 * - Waveform on the clip's content timeline (beats), drawn exactly as the engine maps it
 *   (`clipSourceMapper`), with the warp markers on top.
 * - Markers (warp on): double-click the waveform to add one at that beat (pinned to the
 *   source time currently playing there); drag a marker to move its beat (the audio
 *   stretches; snapped to 1/16, Alt = free); double-click a marker or select it and press
 *   Delete/Backspace to remove it. Every drag (marker or transpose knob) is one undo gesture.
 *   A marker stays between its neighbours, so a drag only re-times the audio between them;
 *   the first marker added to a clip mapped by its source tempo alone also pins the media
 *   start and end, so the audio outside the neighbours never moves. The view keeps its
 *   scale during a drag.
 */

import { useEffect, useMemo, useReducer, useRef, useState, type KeyboardEvent, type PointerEvent, type RefObject } from "react";
import { useShallow } from "zustand/react/shallow";
import type { AudioContent, Beats, Clip, MediaRef, WarpMarker, WarpMarkerId, WarpMode } from "@/generated";
import { Knob, openContextMenu, Select, useScopedDrag, useThemeColor, type SelectOption } from "@/kit";
import { resolveClipEntries } from "@/features/import-resolver/menus";
import { size } from "@/theme";
import { clipDisplayName, useClip, useEditedClipId, useProjectStore, warpMarkersOfClip } from "@/state";
import { createTimelineViewStore, useSelectedItems, useTempoMap } from "@/timeline";
import { ClipEnvelopeStrip } from "@/features/automation";
import { clipTempoMap } from "@/features/piano-roll/clipTime";
import { audioClips, offsetTranspose, setAllTranspose, setAllWarp, transposeBases, type AudioClip, type Bases } from "@/features/clip-editing/multiEdit";
import { cmd, newId, useTransport } from "@/transport";
import { PeakCache, peakLevel, peakRange, TILE_PEAKS } from "@/features/arrangement/peaks";
import { dbToGain, drawClipMarks, waveColumn, type WaveColumn } from "@/features/arrangement/clipDraw";
import { openGesture, sendOne, startDrag } from "./drag";
import { beatAtSource, clipSourceMapper, compileWarp, complexStretches, sourceSecondsAt } from "@/domain/warpMap";
import "./warp.css";
import { formatFixed, useT, type MessageKey } from "@/i18n";

/** Marker snap grid (beats) unless Alt is held. */
export const MARKER_SNAP: Beats = 0.25;
/** Closest two markers may get (beats). */
const MIN_GAP: Beats = 1 / 64;
const WARP_MODES: ReadonlyArray<{ value: WarpMode; label: MessageKey }> = [
  { value: "Repitch", label: "warp.mode.repitch" },
  { value: "Complex", label: "warp.mode.complex" },
];
const TRANSPOSE_RANGE = 48;
const HEIGHT = 140;
const RULER = 16;
const FALLBACK_WIDTH = 800;
/** Height of the clip envelope lane under the waveform. */
const ENVELOPE_HEIGHT = 72;

/** Prop-less warp editor mounted by the app shell. */
export function WarpEditor() {
  const id = useEditedClipId();
  const clip = useClip(id);
  const t = useT();
  if (!clip) return <Empty text={t("warp.empty.no_clip")} />;
  if (clip.content.type !== "Audio") return <Empty text={t("warp.empty.midi_clip")} />;
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
  const t = useT();
  // Unnamed, the clip reads as its track.
  const clipName = useProjectStore((s) => (s.project ? clipDisplayName(s.project, clip) : clip.name));
  const windowDrag = useScopedDrag(startDrag);
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
  const fitted = Math.max(1, contentEnd, mediaEnd) * 1.05;
  // The view fits the clip, except around a marker drag: the drag holds the scale it started
  // with (the markers and waveform outside the dragged marker's neighbours must not move on
  // screen), and afterwards the view only zooms out if the audio no longer fits.
  const [held, setHeld] = useState<{ span: Beats; dragging: boolean } | null>(null);
  const span = held ? (held.dragging ? held.span : Math.max(held.span, fitted)) : fitted;
  const pxPerBeat = width / span;
  const xOf = (b: Beats) => b * pxPerBeat;
  // The clip envelope lane shares the waveform's axis (content beats, fitted to the width).
  const [envView] = useState(() => createTimelineViewStore({ limits: { minPxPerBeat: 1e-6, maxPxPerBeat: 1e6, minScrollBeats: 0 } }));
  useEffect(() => {
    envView.getState().setViewport({ pxPerBeat, scrollBeats: 0 });
    envView.getState().setWidth(width);
  }, [envView, pxPerBeat, width]);
  const envTempo = useMemo(() => clipTempoMap(tempo, clip), [tempo, clip]);

  // With several audio clips selected (this one among them), warp and transpose edits apply
  // to all of them.
  const targets = useEditTargets(clip);
  const setWarp = (patch: Partial<typeof warp>) => {
    const c = setAllWarp(targets, patch);
    if (c) sendOne(transport, c);
  };

  const addMarkerAt = (clientX: number, free: boolean) => {
    if (!warp.enabled) return;
    const rect = boxRef.current?.getBoundingClientRect();
    const beat = Math.max(0, snap((clientX - (rect?.left ?? 0)) / pxPerBeat, free));
    if (markers.some((m) => Math.abs(m.beat - beat) < MIN_GAP)) return;
    const id = newId();
    setSelected(id);
    const at = (b: Beats) => sourceSecondsAt(pins, refBpm, b);
    const add = (mid: WarpMarkerId, b: Beats, source: number) => cmd("Warp", { type: "AddMarker", id: mid, clip: clip.id, beat: b, source });
    const commands = [add(id, beat, at(beat))];
    // With fewer than two markers the source tempo maps the clip (the engine extends past
    // the edge markers with the edge slopes): pin the media start and end first, as they
    // play now, so moving a marker later only re-times the audio between its neighbours.
    if (markers.length < 2 && mediaSeconds > 0) {
      // The mapping is a line here: invert it exactly.
      const [b0, s0] = pins?.[0] ?? [0, 0];
      const [b1, s1] = pins?.[1] ?? [1, 60 / (refBpm > 0 ? refBpm : 120)];
      for (const s of [0, mediaSeconds]) {
        const b = b0 + ((s - s0) * (b1 - b0)) / (s1 - s0);
        if (Number.isFinite(b) && ![beat, ...markers.map((m) => m.beat)].some((x) => Math.abs(x - b) < MIN_GAP)) {
          commands.push(add(newId(), b, s));
        }
      }
    }
    const label = "add warp marker"; // i18n-ignore (the English label: agents, logs)
    sendOne(transport, commands.length === 1 ? commands[0]! : cmd("Edit", { type: "Batch", label, key: { id: "undo.warp.add_marker", args: {} }, commands }));
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
    // For Delete; never scroll the panel to it (that would shift the view under the pointer).
    boxRef.current?.focus({ preventScroll: true });
    const sorted = [...markers].sort((a, b) => a.beat - b.beat);
    const i = sorted.findIndex((x) => x.id === m.id);
    const lo = i > 0 ? sorted[i - 1]!.beat + MIN_GAP : -Infinity;
    const hi = i < sorted.length - 1 ? sorted[i + 1]!.beat - MIN_GAP : Infinity;
    // Hold the scale for the whole drag (`pxPerBeat` below is this one).
    setHeld({ span, dragging: true });
    windowDrag(
      transport,
      e,
      (dx, ev) => {
        if (dx === 0) return null;
        const beat = Math.min(hi, Math.max(lo, snap(m.beat + dx / pxPerBeat, ev.altKey)));
        return cmd("Warp", { type: "MoveMarker", id: m.id, beat, source: m.source });
      },
      () => setHeld({ span, dragging: false }),
    );
  };

  const onKeyDown = (e: KeyboardEvent) => {
    if ((e.key === "Delete" || e.key === "Backspace") && selected) {
      e.preventDefault();
      removeMarker(selected);
    }
  };

  const webFallback = warp.enabled && warp.mode === "Complex" && !complexStretches(transport.kind);

  return (
    <div
      className="eth-warp"
      data-feature="warp"
      data-testid="warp-editor"
      data-clip-id={clip.id}
      // "Resolve…" when the clip's sample is missing.
      onContextMenu={(e) => {
        const entries = resolveClipEntries(clip);
        if (entries.length > 0) openContextMenu(e, entries);
      }}
    >
      <div className="eth-warp__toolbar">
        <span className="eth-warp__title eth-user-text" title={clipName}>
          {clipName}
        </span>
        <label className="eth-warp__field">
          <input type="checkbox" aria-label={t("warp.toggle")} checked={warp.enabled} onChange={(e) => setWarp({ enabled: e.target.checked })} />
          {t("warp.toggle")}
        </label>
        <span className="eth-warp__field">
          {t("warp.mode_label")}
          <Select<WarpMode>
            size="sm"
            aria-label={t("warp.mode_aria")}
            value={warp.mode}
            disabled={!warp.enabled}
            options={WARP_MODES.map((m): SelectOption<WarpMode> => ({ value: m.value, label: t(m.label) }))}
            onChange={(mode) => setWarp({ mode })}
          />
        </span>
        {webFallback && (
          <span className="eth-warp__note" data-testid="warp-web-fallback">
            {t("warp.web_fallback")}
          </span>
        )}
        <TransposeControl targets={targets} semitones={content.transpose} />
        <span className="eth-warp__info" data-testid="warp-source-bpm">
          {warp.source_bpm ? t("warp.source_bpm", { bpm: formatFixed(warp.source_bpm, 2) }) : t("warp.source_unknown")}
        </span>
        <span className="eth-warp__info" data-testid="warp-marker-count">
          {warp.enabled ? t("warp.markers", { count: markers.length }) : t("warp.unwarped")}
        </span>
      </div>
      <div
        className="eth-warp__view"
        ref={boxRef}
        tabIndex={0}
        role="group"
        aria-label={t("warp.view")}
        style={{ height: HEIGHT }}
        data-testid="warp-view"
        data-px-per-beat={pxPerBeat}
        onKeyDown={onKeyDown}
        onPointerDown={() => setSelected(null)}
        onDoubleClick={(e) => addMarkerAt(e.clientX, e.altKey)}
      >
        {media && (
          <Waveform media={media} width={width} span={span} toSeconds={toSeconds} playedFrom={clip.offset} playedTo={contentEnd} gainDb={content.gain} />
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
                <title>{t("warp.marker_title", { beat: formatFixed(m.beat, 3), source: formatFixed(m.source, 3) })}</title>
              </g>
            ))}
        </svg>
      </div>
      <ClipEnvelopeStrip clip={clip} view={envView} height={ENVELOPE_HEIGHT} tempo={envTempo} />
    </div>
  );
}

/**
 * The audio clips an edit in this editor applies to: every selected audio clip when `clip`
 * is one of them, else `clip` alone.
 */
function useEditTargets(clip: Clip): AudioClip[] {
  const selected = useSelectedItems("clip");
  const inSelection = selected.size > 1 && selected.has(clip.id);
  const others = useProjectStore(
    useShallow((s) => (inSelection && s.project ? [...selected].flatMap((id) => (s.project!.clips[id] ? [s.project!.clips[id]] : [])) : [])),
  );
  const targets = inSelection ? audioClips(others) : [];
  return targets.some((t) => t.clip.id === clip.id) ? targets : audioClips([clip]);
}

function TransposeControl({ targets, semitones }: { targets: AudioClip[]; semitones: number }) {
  const t = useT();
  const transport = useTransport();
  const gesture = useRef<{ g: ReturnType<typeof openGesture>; bases: Bases; from: number } | null>(null);
  const clampSt = (st: number) => Math.max(-TRANSPOSE_RANGE, Math.min(TRANSPOSE_RANGE, st));
  return (
    <span className="eth-warp__field eth-warp__transpose">
      <Knob
        value={(semitones + TRANSPOSE_RANGE) / (2 * TRANSPOSE_RANGE)}
        bipolar
        size={parseFloat(size.knobSm)}
        label={t("warp.transpose")}
        valueText={t("warp.semitones", { value: `${semitones > 0 ? "+" : ""}${semitones}` })}
        onChangeStart={() => {
          gesture.current = { g: openGesture(transport), bases: transposeBases(targets), from: semitones };
        }}
        onChange={(v) => {
          const st = clampSt(Math.round(v * 2 * TRANSPOSE_RANGE - TRANSPOSE_RANGE));
          // The knob moves every target by the same amount (their differences are kept).
          // Inside a drag: part of its gesture. Keyboard/wheel/reset: one undo step each.
          const d = gesture.current;
          const c = offsetTranspose(targets, d?.bases ?? transposeBases(targets), st - (d?.from ?? semitones));
          if (!c) return;
          if (d) d.g.send(c);
          else sendOne(transport, c);
        }}
        onChangeEnd={() => {
          gesture.current?.g.end();
          gesture.current = null;
        }}
      />
      <input
        type="number"
        aria-label={t("warp.transpose")}
        className="eth-warp__number"
        min={-TRANSPOSE_RANGE}
        max={TRANSPOSE_RANGE}
        step={1}
        value={semitones}
        onChange={(e) => {
          // A typed value: every target gets it.
          const v = Number(e.target.value);
          if (!Number.isFinite(v)) return;
          const c = setAllTranspose(targets, clampSt(v));
          if (c) sendOne(transport, c);
        }}
      />
      {t("warp.semitones_unit")}
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
  gainDb,
}: {
  media: MediaRef;
  width: number;
  span: Beats;
  toSeconds: (c: Beats) => number;
  /** Content range the clip plays (drawn brighter). */
  playedFrom: Beats;
  playedTo: Beats;
  /** Clip gain (dB): the cached peaks are scaled by it at draw time, like the arrangement. */
  gainDb: number;
}) {
  const t = useT();
  const transport = useTransport();
  const peaks = useMemo(() => new PeakCache(transport), [transport]);
  const ref = useRef<HTMLCanvasElement>(null);
  const [tick, redraw] = useReducer((x: number) => x + 1, 0);
  useEffect(() => peaks.subscribe(redraw), [peaks]);
  const waveColor = useThemeColor("warpWave");
  const waveDim = useThemeColor("warpWaveDim");
  const over = useThemeColor("meterHigh");

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
    // Detail level from the mean frames per column over the view, not one column's: moving
    // a marker must not redraw the audio outside its neighbours at another level.
    const level = peakLevel((Math.abs(toSeconds(span) - toSeconds(0)) * sr) / Math.max(1, width));
    const tile = (i: number) => (i * TILE_PEAKS * level < media.frames ? peaks.tile(media.id, level, i) : null);
    const mid = h / 2;
    const gain = dbToGain(gainDb);
    const marks: Array<[number, WaveColumn]> = [];
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
        const col = waveColumn(p.min, p.max, gain, mid);
        if (col.clipTop || col.clipBottom) marks.push([x, col]);
        ctx.rect(x, col.top, 1, Math.max(1, col.bottom - col.top));
      }
      ctx.fill();
    }
    drawClipMarks(ctx, marks, h, over);
    canvas.dataset.clipped = String(marks.length);
  }, [media, width, span, toSeconds, playedFrom, playedTo, gainDb, peaks, tick, waveColor, waveDim, over]);

  // A picture of the audio (the warp markers over it are titled and counted in the toolbar).
  return (
    <canvas
      ref={ref}
      className="eth-warp__canvas"
      style={{ width, height: HEIGHT - RULER, top: RULER }}
      data-testid="warp-waveform"
      data-gain={gainDb}
      role="img"
      aria-label={t("warp.waveform")}
    />
  );
}
