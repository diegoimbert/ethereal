/**
 * Warp editor: the detail tab for the audio clip open in the detail editor
 * (`useEditedClipId()`, set by double-clicking a clip in the arrangement).
 *
 * - Toolbar: the warp/pitch mode (none = warp off | Repitch | Complex | Complex+), transpose (semitones), source
 *   tempo, and for Complex+ its formant settings (`FormantFields`). Complex and Complex+ keep
 *   pitch in every build (the browser runs pure-Rust stretchers); should an engine lack a
 *   stretcher, the toolbar says the clip plays as Repitch.
 *   With several audio clips selected (the open one among them) the toolbar says
 *   "N audio clips selected" and its edits apply to all of them (relative drags, see
 *   `TransposeControl`); values they don't share show as mixed.
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
import type { AudioContent, Beats, Clip, MediaRef, WarpMarker, WarpMarkerId } from "@/generated";
import { Knob, NumberField, openContextMenu, Select, useScopedDrag, useThemeColor, type SelectOption } from "@/kit";
import { resolveClipEntries } from "@/features/import-resolver/menus";
import { size } from "@/theme";
import { clipDisplayName, useClip, useEditedClipId, useProjectStore, warpMarkersOfClip } from "@/state";
import { createTimelineViewStore, useSelectedItems, useTempoMap } from "@/timeline";
import { ClipEnvelopeStrip } from "@/features/automation";
import { clipTempoMap } from "@/features/piano-roll/clipTime";
import {
  audioClips,
  common,
  offsetTranspose,
  setAllTranspose,
  setAllWarpChoice,
  TRANSPOSE_MAX,
  transposeBases,
  WARP_CHOICES,
  warpChoiceOf,
  type AudioClip,
  type Bases,
  type WarpChoice,
} from "@/features/clip-editing/multiEdit";
import { cmd, newId, useTransport } from "@/transport";
import { PeakCache, peakLevel, peakRange, TILE_PEAKS } from "@/features/arrangement/peaks";
import { dbToGain, drawClipMarks, waveColumn, type WaveColumn } from "@/features/arrangement/clipDraw";
import { openGesture, sendOne, startDrag } from "./drag";
import { FormantFields } from "./FormantFields";
import { beatAtSource, clipSourceMapper, compileWarp, complexStretches, sourceSecondsAt } from "@/domain/warpMap";
import "./warp.css";
import { formatFixed, useT } from "@/i18n";

/** Marker snap grid (beats) unless Alt is held. */
export const MARKER_SNAP: Beats = 0.25;
/** Closest two markers may get (beats). */
const MIN_GAP: Beats = 1 / 64;
const TRANSPOSE_RANGE = TRANSPOSE_MAX;
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

  const webFallback = warp.enabled && warp.mode !== "Repitch" && !complexStretches(transport.kind);
  // Several clips: a mode they don't share shows as "—" (none, warp off, is one of them).
  const choice = common(targets.map((a) => warpChoiceOf(a.audio.warp)));

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
        {targets.length > 1 && (
          <span className="eth-warp__clip-count" data-testid="warp-clip-count" title={t("warp.clip_count_title")}>
            {t("warp.clip_count", { count: targets.length })}
          </span>
        )}
        <span className="eth-warp__field">
          {t("warp.mode_label")}
          <Select<WarpChoice>
            size="sm"
            aria-label={t("warp.mode_aria")}
            value={(choice ?? "") as WarpChoice}
            placeholder="—"
            options={WARP_CHOICES.map((m): SelectOption<WarpChoice> => ({ value: m.value, label: t(m.label) }))}
            onChange={(c) => {
              const command = setAllWarpChoice(targets, c);
              if (command) sendOne(transport, command);
            }}
          />
        </span>
        {webFallback && (
          <span className="eth-warp__note" data-testid="warp-web-fallback">
            {t("warp.web_fallback")}
          </span>
        )}
        <TransposeControl targets={targets} semitones={content.transpose} />
        {choice === "ComplexPlus" && (
          <FormantFields
            targets={targets}
            row={(key, label, title, field) => (
              <span key={key} className="eth-warp__field" title={title}>
                {label}
                {field}
              </span>
            )}
          />
        )}
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
      <ClipEnvelopeStrip
        clip={clip}
        view={envView}
        height={ENVELOPE_HEIGHT}
        tempo={envTempo}
        backdrop={
          media && (
            <Waveform
              media={media}
              width={width}
              span={span}
              toSeconds={toSeconds}
              playedFrom={clip.offset}
              playedTo={contentEnd}
              gainDb={content.gain}
              height={ENVELOPE_HEIGHT}
              top={0}
              testId="clip-envelope-waveform"
            />
          )
        }
      />
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

/**
 * Transpose (knob + field) of the edited clip, or of every target: a drag, an arrow step or a
 * wheel turn moves each target by the same amount from its own value (−6 and +3 dragged up 2
 * become −4 and +5; each clamped to ±48 on its own), a typed value sets them all. Values that
 * differ show as mixed ("—"), and as the offset applied while a drag runs.
 */
function TransposeControl({ targets, semitones }: { targets: AudioClip[]; semitones: number }) {
  const t = useT();
  const transport = useTransport();
  const gesture = useRef<{ g: ReturnType<typeof openGesture>; bases: Bases; from: number } | null>(null);
  // The offset of the running knob drag (shown in place of "mixed").
  const [offset, setOffset] = useState<number | null>(null);
  const clampSt = (st: number) => Math.max(-TRANSPOSE_RANGE, Math.min(TRANSPOSE_RANGE, st));
  const mixed = targets.length > 1 && common(targets.map((a) => a.audio.transpose)) === null;
  const signed = (st: number) => `${st > 0 ? "+" : ""}${st}`;
  const valueText = mixed
    ? offset !== null
      ? t("warp.semitones", { value: signed(offset) })
      : t("kit.number.mixed")
    : t("warp.semitones", { value: signed(semitones) });
  /** Every target moved by `delta` from `bases`: inside a drag part of its gesture, else one undo step. */
  const nudge = (delta: number, bases: Bases) => {
    const c = offsetTranspose(targets, bases, delta);
    if (!c) return;
    if (gesture.current) gesture.current.g.send(c);
    else sendOne(transport, c);
  };
  const start = () => {
    gesture.current = { g: openGesture(transport), bases: transposeBases(targets), from: semitones };
  };
  const end = () => {
    gesture.current?.g.end();
    gesture.current = null;
    setOffset(null);
  };
  return (
    <span className="eth-warp__field eth-warp__transpose">
      <Knob
        value={(semitones + TRANSPOSE_RANGE) / (2 * TRANSPOSE_RANGE)}
        bipolar
        size={parseFloat(size.knobSm)}
        label={t("warp.transpose")}
        valueText={valueText}
        onChangeStart={start}
        onChange={(v) => {
          const st = clampSt(Math.round(v * 2 * TRANSPOSE_RANGE - TRANSPOSE_RANGE));
          // Keyboard/wheel/reset (no drag): a step from the current values.
          const d = gesture.current;
          if (d) setOffset(st - d.from);
          nudge(st - (d?.from ?? semitones), d?.bases ?? transposeBases(targets));
        }}
        onChangeEnd={end}
      />
      <NumberField
        size="sm"
        aria-label={t("warp.transpose")}
        className="eth-warp__number"
        min={-TRANSPOSE_RANGE}
        max={TRANSPOSE_RANGE}
        value={semitones}
        mixed={mixed}
        onChangeStart={start}
        onChangeEnd={end}
        // A typed value: every target gets it.
        onChange={(v) => {
          const c = setAllTranspose(targets, clampSt(Math.round(v)));
          if (c) sendOne(transport, c);
        }}
        onNudge={(d) => nudge(d, gesture.current?.bases ?? transposeBases(targets))}
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
  height = HEIGHT - RULER,
  top = RULER,
  testId = "warp-waveform",
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
  /** Size and place (px; default: the warp view's, under its ruler). */
  height?: number;
  top?: number;
  testId?: string;
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
    const h = height;
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
  }, [media, width, span, toSeconds, playedFrom, playedTo, gainDb, height, peaks, tick, waveColor, waveDim, over]);

  // A picture of the audio (the warp markers over it are titled and counted in the toolbar).
  return (
    <canvas
      ref={ref}
      className="eth-warp__canvas"
      style={{ width, height, top }}
      data-testid={testId}
      data-gain={gainDb}
      role="img"
      aria-label={t("warp.waveform")}
    />
  );
}
